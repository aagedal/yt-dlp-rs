mod formats;
mod template;
pub use formats::render_formats;

use crate::{Result, VideoInfo};
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

/// Options for the supported filename template conversions.
#[derive(Debug, Clone)]
pub struct TemplateOptions {
    /// Used for unavailable metadata or failed numeric conversion.
    pub na_placeholder: String,
}

impl Default for TemplateOptions {
    fn default() -> Self {
        Self {
            na_placeholder: "NA".into(),
        }
    }
}

/// Render a supported typed template with the default missing-value placeholder.
/// Only literal template text can introduce directories.
pub fn render_template(template: &str, info: &VideoInfo) -> Result<PathBuf> {
    render_template_with_options(template, info, &TemplateOptions::default())
}

/// Render supported typed metadata with explicit missing-value options.
pub fn render_template_with_options(
    template: &str,
    info: &VideoInfo,
    options: &TemplateOptions,
) -> Result<PathBuf> {
    template::render(template, info, options)
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
