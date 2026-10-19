mod exec;
mod filename;
mod formats;
mod links;
mod print;
pub mod progress;
mod sanitize;
mod subtitles;
mod template;
mod thumbnails;
pub use exec::{ExecCommand, ExecCommands, Stage};
pub use filename::{OUTPUT_TYPES, OutputTemplates, parse_paths, replace_extension};
pub use formats::render_formats;
pub use links::{iri_to_uri, link_file};
pub use print::{PrintTemplate, PrintToFile, StagePaths};

/// The case-insensitive `WHEN:` stage name of a `--print` value (`video`
/// when absent), as upstream's `when_prefix` splits it.
pub fn print_stage_name(spec: &str) -> String {
    exec::Stage::split(spec, "video").0
}
pub use sanitize::{FilenameMode, force_sanitize_path, sanitize_filename};
pub use subtitles::{render_automatic_captions, render_subtitles};
pub use thumbnails::render_thumbnails;

use crate::{PlaylistCounts, PlaylistInfo, Result, VideoInfo};
use std::path::PathBuf;

pub const DEFAULT_TEMPLATE: &str = "%(title)s [%(id)s].%(ext)s";

/// Upstream `_parse_outtmpl`: restricted names remove spaces in the default template.
pub fn default_template(restricted: bool) -> String {
    if restricted {
        DEFAULT_TEMPLATE.replace(" - ", " ").replace(' ', "-")
    } else {
        DEFAULT_TEMPLATE.into()
    }
}

pub(crate) fn safe_component(value: &str) -> String {
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
    /// Metadata sanitization (`--restrict-filenames`/`--no-windows-filenames`).
    pub filename_mode: FilenameMode,
    /// `--windows-filenames`: force upstream Windows path sanitization.
    pub windows_filenames: bool,
    /// `--trim-filenames`: Python-slice length excluding extensions; 0 disables.
    pub trim_file_name: i64,
    /// `--paths`: `home` and output-type directories joined before names.
    pub paths: std::collections::BTreeMap<String, String>,
}

impl TemplateOptions {
    /// Resolve upstream's restrict/windows filename options for this platform.
    pub fn filename_mode(restrict: bool, windows: Option<bool>) -> FilenameMode {
        if restrict {
            FilenameMode::Restricted
        } else if !cfg!(windows) && windows == Some(false) {
            FilenameMode::Minimal
        } else {
            FilenameMode::Default
        }
    }
}

impl Default for TemplateOptions {
    fn default() -> Self {
        Self {
            na_placeholder: "NA".into(),
            filename_mode: FilenameMode::Default,
            windows_filenames: false,
            trim_file_name: 0,
            paths: Default::default(),
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
    template::render(template, template::Source::Video(info), options)
}

/// Render a playlist-level file name, as upstream's playlist info JSON does.
/// Playlist metadata supplies id/title/extractor/webpage URL plus
/// `playlist_index` 0 and the counts; other represented fields are missing.
pub fn render_playlist_template_with_options(
    template: &str,
    info: &PlaylistInfo,
    counts: PlaylistCounts,
    options: &TemplateOptions,
) -> Result<PathBuf> {
    let source = template::Source::Playlist {
        info,
        playlist_count: counts.playlist_count,
        n_entries: counts.n_entries,
        last_playlist_index: counts.last_playlist_index,
    };
    template::render(template, source, options)
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
