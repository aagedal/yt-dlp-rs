use crate::{Error, Result, VideoInfo};
use std::path::PathBuf;

pub const DEFAULT_TEMPLATE: &str = "%(title)s [%(id)s].%(ext)s";

fn safe_component(value: &str) -> String {
    let value: String = value
        .chars()
        .map(|c| {
            if c.is_control() || "/\\<>:\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let value = value.trim().trim_matches('.');
    if value.is_empty() {
        "video".into()
    } else {
        value.into()
    }
}

/// Only literal directory paths and the documented string fields are supported.
/// Remote metadata is sanitized before substitution and cannot create directories.
pub fn render_template(template: &str, info: &VideoInfo) -> Result<PathBuf> {
    let mut output = String::new();
    let mut rest = template;
    while let Some(index) = rest.find('%') {
        output.push_str(&rest[..index]);
        rest = &rest[index + 1..];
        if let Some(tail) = rest.strip_prefix('%') {
            output.push('%');
            rest = tail;
            continue;
        }
        let Some((field, tail)) = rest.strip_prefix('(').and_then(|s| s.split_once(")s")) else {
            return Err(Error::InvalidTemplate(template.into()));
        };
        let value = match field {
            "id" => &info.id,
            "title" => &info.title,
            "ext" => &info.ext,
            "extractor" => &info.extractor,
            _ => return Err(Error::InvalidTemplate(format!("unknown field {field:?}"))),
        };
        output.push_str(&safe_component(value));
        rest = tail;
    }
    output.push_str(rest);
    if output.is_empty() || output == "-" {
        return Err(Error::InvalidTemplate(
            "empty paths and stdout output are unsupported".into(),
        ));
    }
    Ok(output.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_names_cannot_create_paths() {
        assert_eq!(safe_component("../../escape\\bad\n"), "_.._escape_bad_");
        assert_eq!(safe_component(".."), "video");
    }
}
