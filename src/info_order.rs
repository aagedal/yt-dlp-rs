//! Upstream info-dict key order and `None` keys for printed and written info
//! (`-j`, `-J`, info JSON). Python dicts keep insertion order, so upstream's
//! order is the order its code paths add keys: the extractor's dict, then
//! `add_extra_info`, `process_video_result` and the selected format's
//! `info_dict.update(format)`. Extractors mark which upstream path built an
//! info or format dict; [`video`] and [`format`] replay that path, keeping
//! this port's values and adding `null` where upstream assigns `None`.
//! Unmarked dicts (YouTube, loaded info JSON) keep this port's order.
use serde_json::{Map, Value};

/// The upstream code path of an info dict (see [`video`]).
pub(crate) const INFO_SHAPE: &str = "__yt_dlp_rs_info_shape";
/// The upstream code path of a format dict (see [`format`]).
pub(crate) const FORMAT_SHAPE: &str = "__yt_dlp_rs_format_shape";

/// How an upstream step sets a key.
#[derive(Clone, Copy)]
enum Set {
    /// Only with a value.
    Take,
    /// Always, `None` without a value.
    Assign,
    /// Always, `{}` without a value.
    Dict,
    /// `parse_codecs`'s `dynamic_range`: always when codecs are known.
    Codecs,
}
use Set::*;

/// Insertion replay: keys move from this port's dict into upstream order.
struct Replay {
    ours: Map<String, Value>,
    out: Map<String, Value>,
}

impl Replay {
    fn new(ours: Map<String, Value>) -> Self {
        Self {
            ours,
            out: Map::new(),
        }
    }

    /// A key upstream sets when it has a value.
    fn take(&mut self, key: &str) {
        if let Some(value) = self.ours.remove(key) {
            self.out.insert(key.to_owned(), value);
        }
    }

    /// A key upstream always assigns, `None` without a value. An already
    /// placed key keeps its position.
    fn assign(&mut self, key: &str) {
        match self.ours.remove(key) {
            Some(value) => {
                self.out.insert(key.to_owned(), value);
            }
            None if !self.out.contains_key(key) => {
                self.out.insert(key.to_owned(), Value::Null);
            }
            None => {}
        }
    }

    fn set(&mut self, key: &str, how: Set) {
        match how {
            Take => self.take(key),
            Assign => self.assign(key),
            Dict => {
                if !self.ours.contains_key(key) && !self.out.contains_key(key) {
                    self.out.insert(key.to_owned(), Value::Object(Map::new()));
                }
                self.take(key);
            }
            Codecs => {
                if self.get("vcodec").is_some() || self.get("acodec").is_some() {
                    self.assign(key);
                }
            }
        }
    }

    fn steps(&mut self, steps: &[(&str, Set)]) {
        for (key, how) in steps {
            self.set(key, *how);
        }
    }

    /// Remaining keys, except those `held` for later steps.
    fn rest_except(&mut self, held: &dyn Fn(&str) -> bool) {
        let keys: Vec<String> = self.ours.keys().filter(|key| !held(key)).cloned().collect();
        for key in keys {
            self.take(&key);
        }
    }

    fn finish(mut self) -> Map<String, Value> {
        self.rest_except(&|_| false);
        self.out
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.ours.get(key).or_else(|| self.out.get(key))
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// Python truthiness of a value.
    fn truthy(&self, key: &str) -> bool {
        match self.get(key) {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(Value::Number(number)) => number.as_f64() != Some(0.0),
            Some(Value::String(text)) => !text.is_empty(),
            Some(Value::Array(items)) => !items.is_empty(),
            Some(Value::Object(map)) => !map.is_empty(),
        }
    }

    /// `process_video_result`'s per-format steps: `_fill_sorting_fields`,
    /// the audio `acodec`, `resolution`, `dynamic_range`, `aspect_ratio`,
    /// `filesize_approx` and `http_headers`, then `format_id` and `format`.
    fn format_steps(&mut self) {
        self.take("url");
        self.assign("protocol");
        self.assign("ext");
        let audio_only = self.text("vcodec") == Some("none");
        if audio_only {
            self.assign("audio_ext");
            self.assign("video_ext");
        } else {
            self.assign("video_ext");
            self.assign("audio_ext");
        }
        self.take("preference");
        // `vbr`/`abr` are zeroed for a missing stream before the others.
        if audio_only {
            self.assign("vbr");
        }
        if self.text("acodec") == Some("none") {
            self.assign("abr");
        }
        for key in ["vbr", "abr", "tbr"] {
            self.assign(key);
        }
        self.take("acodec");
        self.assign("resolution");
        if !audio_only {
            self.assign("dynamic_range");
        }
        self.assign("aspect_ratio");
        // `filesize_from_tbr` (see `info::fill_format_descriptions`), `None`
        // without a bitrate or duration.
        let manifests =
            crate::info::COMPAT_MANIFEST_FILESIZE_APPROX.load(std::sync::atomic::Ordering::Relaxed);
        if (manifests || self.get("manifest_url").is_none())
            && !self.truthy("filesize")
            && !self.truthy("filesize_approx")
        {
            self.assign("filesize_approx");
        } else {
            self.take("filesize_approx");
        }
        self.assign("http_headers");
        self.assign("format_id");
        self.assign("format");
    }
}

/// Drop the code-path markers from a dict other code reads as upstream's.
pub(crate) fn strip_markers(map: &mut Map<String, Value>) {
    map.remove(INFO_SHAPE);
    map.remove(FORMAT_SHAPE);
}

/// Record the upstream code path that built an info dict.
pub(crate) fn mark_info(mut info: crate::VideoInfo, shape: &str) -> crate::VideoInfo {
    info.extra.insert(INFO_SHAPE.into(), shape.into());
    info
}

/// Record the upstream code path that built a format dict.
pub(crate) fn mark_format(format: &mut crate::VideoFormat, shape: &str) {
    format.extra.insert(FORMAT_SHAPE.into(), shape.into());
}

/// One format dict in upstream order (unmarked formats are unchanged).
pub(crate) fn format(mut map: Map<String, Value>) -> Map<String, Value> {
    let Some(Value::String(shape)) = map.remove(FORMAT_SHAPE) else {
        return map;
    };
    // `_parse_html5_media_entries` adds the page Referer to every format.
    let (html5, shape) = match shape.strip_prefix("html5+") {
        Some(shape) => (true, shape),
        None => (shape.starts_with("html5-"), shape.as_str()),
    };
    let mut replay = Replay::new(map);
    let extractor: &[(&str, Set)] = match shape {
        // GenericIE's direct link: `vcodec` is `none` for audio, else `None`.
        "direct" => &[
            ("format_id", Take),
            ("url", Take),
            ("ext", Take),
            ("vcodec", Assign),
        ],
        // `_media_formats` for a media element's `src`.
        "html5-src" => &[("url", Take), ("vcodec", Assign), ("ext", Take)],
        // `parse_content_type` codecs and `ext`, the `<source>` attributes,
        // then `f.update(formats[0])`.
        "html5-source" => &[
            ("vcodec", Take),
            ("acodec", Take),
            ("dynamic_range", Codecs),
            ("ext", Take),
            ("width", Assign),
            ("height", Assign),
            ("tbr", Assign),
            ("format_id", Assign),
            ("url", Take),
            ("vcodec", Assign),
            ("ext", Take),
        ],
        // `_extract_m3u8_formats` for a media playlist.
        "hls-media" => &[
            ("format_id", Take),
            ("format_index", Assign),
            ("url", Take),
            ("ext", Take),
            ("protocol", Take),
            ("preference", Assign),
            ("quality", Assign),
            ("has_drm", Take),
        ],
        // A master playlist's variant, with `RESOLUTION` and `parse_codecs`.
        "hls-variant" => &[
            ("format_id", Take),
            ("format_index", Assign),
            ("url", Take),
            ("manifest_url", Take),
            ("tbr", Assign),
            ("ext", Take),
            ("fps", Assign),
            ("protocol", Take),
            ("preference", Assign),
            ("quality", Assign),
            ("has_drm", Take),
            ("width", Take),
            ("height", Take),
            ("vcodec", Take),
            ("acodec", Take),
            ("dynamic_range", Codecs),
        ],
        // A master playlist's audio rendition.
        "hls-rendition" => &[
            ("format_id", Take),
            ("format_note", Take),
            ("format_index", Assign),
            ("url", Take),
            ("manifest_url", Take),
            ("language", Assign),
            ("ext", Take),
            ("protocol", Take),
            ("preference", Assign),
            ("quality", Assign),
            ("has_drm", Take),
            ("vcodec", Assign),
            ("source_preference", Assign),
        ],
        // `_parse_mpd_formats` for a text representation, then its URL or
        // fragments.
        "dash-text" => {
            replay.steps(&[
                ("ext", Take),
                ("manifest_url", Take),
                ("filesize", Assign),
                ("url", Take),
                ("fragment_base_url", Take),
                ("fragments", Take),
                ("protocol", Take),
            ]);
            return replay.finish();
        }
        // `_parse_mpd_formats` for a segmented representation.
        "dash" => &[
            ("format_id", Take),
            ("manifest_url", Take),
            ("ext", Take),
            ("width", Assign),
            ("height", Assign),
            ("tbr", Assign),
            ("asr", Assign),
            ("fps", Assign),
            ("language", Assign),
            ("format_note", Take),
            ("filesize", Assign),
            ("container", Take),
            ("vcodec", Take),
            ("acodec", Take),
            ("dynamic_range", Codecs),
            ("url", Take),
            ("fragment_base_url", Take),
            ("fragments", Take),
            ("protocol", Take),
            ("manifest_stream_number", Take),
            ("is_dash_periods", Take),
        ],
        _ => &[],
    };
    replay.steps(extractor);
    if html5 {
        replay.take("http_headers");
    }
    // Keys this port has beyond the extractor's, before processing.
    replay.rest_except(&|key| PROCESSED_FORMAT_KEYS.contains(&key));
    replay.format_steps();
    replay.finish()
}

/// Keys `process_video_result` adds to formats.
const PROCESSED_FORMAT_KEYS: &[&str] = &[
    "url",
    "protocol",
    "ext",
    "video_ext",
    "audio_ext",
    "vbr",
    "abr",
    "tbr",
    "acodec",
    "resolution",
    "dynamic_range",
    "aspect_ratio",
    "filesize_approx",
    "http_headers",
    "format_id",
    "format",
];

/// Top-level keys added after the extractor's dict.
const PROCESSED_INFO_KEYS: &[&str] = &[
    "webpage_url",
    "original_url",
    "webpage_url_basename",
    "webpage_url_domain",
    "extractor",
    "extractor_key",
    "playlist",
    "playlist_index",
    "playlist_count",
    "playlist_id",
    "playlist_title",
    "playlist_uploader",
    "playlist_uploader_id",
    "playlist_channel",
    "playlist_channel_id",
    "playlist_webpage_url",
    "n_entries",
    "__last_playlist_index",
    "playlist_autonumber",
    "thumbnail",
    "thumbnails",
    "display_id",
    "fulltitle",
    "duration_string",
    "upload_date",
    "release_date",
    "modified_date",
    "release_year",
    "live_status",
    "is_live",
    "was_live",
    "requested_subtitles",
    "_has_drm",
    "epoch",
    "requested_formats",
    "requested_downloads",
    "_filename",
    "filename",
    "__postprocessors",
    "__real_download",
    "__files_to_merge",
    "__finaldir",
    "filepath",
    "__write_download_archive",
    "_type",
    "_version",
];

/// `_merge`'s dict for two selected formats.
const MERGED_KEYS: &[&str] = &[
    "requested_formats",
    "format",
    "format_id",
    "ext",
    "protocol",
    "language",
    "format_note",
    "filesize_approx",
    "tbr",
];
const MERGED_VIDEO_KEYS: &[&str] = &[
    "width",
    "height",
    "resolution",
    "fps",
    "dynamic_range",
    "vcodec",
    "vbr",
    "stretched_ratio",
    "aspect_ratio",
];
const MERGED_AUDIO_KEYS: &[&str] = &["acodec", "abr", "asr", "audio_channels"];

/// One video info dict in upstream order (unmarked dicts are unchanged).
/// Formats must already be ordered by [`format`].
pub(crate) fn video(mut map: Map<String, Value>) -> Map<String, Value> {
    map.remove(FORMAT_SHAPE);
    let Some(Value::String(shape)) = map.remove(INFO_SHAPE) else {
        return map;
    };
    // The selected format's keys, which `info_dict.update(format)` merges.
    let merged = map
        .get("requested_formats")
        .and_then(Value::as_array)
        .is_some_and(|formats| !formats.is_empty());
    // An untyped result is its own (only) format.
    let own_format = shape == "generic-untyped";
    let selected_format: Map<String, Value> = if merged || own_format {
        Map::new()
    } else {
        let id = map.get("format_id").cloned();
        map.get("formats")
            .and_then(Value::as_array)
            .and_then(|formats| {
                formats
                    .iter()
                    .find(|format| format.get("format_id") == id.as_ref())
            })
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    };
    let selected: Vec<String> = if merged {
        let formats = map["requested_formats"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let streams = |codec: &str| {
            formats
                .iter()
                .filter(|format| format.get(codec).and_then(Value::as_str) != Some("none"))
                .count()
        };
        let mut keys: Vec<&str> = MERGED_KEYS.to_vec();
        if streams("vcodec") == 1 {
            keys.extend(MERGED_VIDEO_KEYS);
        }
        if streams("acodec") == 1 {
            keys.extend(MERGED_AUDIO_KEYS);
        }
        keys.into_iter().map(str::to_owned).collect()
    } else if own_format {
        Vec::new()
    } else {
        let id = map.get("format_id").cloned();
        map.get("formats")
            .and_then(Value::as_array)
            .and_then(|formats| {
                formats
                    .iter()
                    .find(|format| format.get("format_id") == id.as_ref())
            })
            .and_then(Value::as_object)
            .map(|format| format.keys().cloned().collect())
            .unwrap_or_default()
    };
    let untyped = shape == "generic-untyped";
    let entry = shape == "html5-entry";
    let mut replay = Replay::new(map);
    let extractor: &[(&str, Set)] = match shape.as_str() {
        "generic-direct" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("direct", Take),
            ("formats", Take),
            ("subtitles", Dict),
            ("http_headers", Assign),
            ("hls_aes", Assign),
        ],
        // "URL could be a direct video link": the info is its own format.
        "generic-untyped" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("direct", Take),
            ("url", Take),
            ("ext", Take),
        ],
        // A typed (Content-Type) HLS link; `_extra_manifest_info` adds
        // `hls_aes` and the "Checking m3u8 live status" duration.
        "generic-hls" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("formats", Take),
            ("subtitles", Dict),
            ("http_headers", Assign),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Assign),
            ("live_status", Take),
            ("duration", Take),
        ],
        // An `#EXTM3U` body without an HLS Content-Type.
        "generic-m3u" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("formats", Take),
            ("subtitles", Dict),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Assign),
            ("live_status", Take),
            ("duration", Take),
        ],
        "generic-dash" => &[
            ("id", Assign),
            ("title", Assign),
            ("timestamp", Assign),
            ("formats", Take),
            ("subtitles", Dict),
            ("live_status", Assign),
            ("extra_param_to_segment_url", Take),
            ("extra_param_to_key_url", Take),
            ("hls_aes", Assign),
        ],
        // `merge_dicts(embed, info_dict)` drops the embed's `None` values.
        "html5" => &[
            ("formats", Take),
            ("subtitles", Dict),
            ("thumbnail", Take),
            ("id", Take),
            ("title", Take),
            ("_old_archive_ids", Take),
            ("extractor", Take),
            ("extractor_key", Take),
            ("timestamp", Take),
            ("description", Take),
            ("thumbnail", Take),
            ("age_limit", Take),
        ],
        "html5-entry" => &[
            ("formats", Take),
            ("subtitles", Dict),
            ("thumbnail", Assign),
            ("id", Take),
            ("title", Take),
            ("_old_archive_ids", Take),
            ("extractor", Take),
            ("extractor_key", Take),
        ],
        _ => &[],
    };
    if untyped {
        // Upstream has no `formats`: the info dict is the only format.
        replay.ours.remove("formats");
    }
    replay.steps(extractor);
    let held = |key: &str| {
        PROCESSED_INFO_KEYS.contains(&key)
            || (untyped && PROCESSED_FORMAT_KEYS.contains(&key))
            || selected.iter().any(|selected| selected == key)
    };
    replay.rest_except(&held);
    // `add_extra_info`: the URL fields, or a playlist entry's
    // `ChainMap(entry extras, _playlist_infodict(...))` keys.
    if entry {
        for (key, always) in [
            ("playlist_count", Take),
            ("playlist", Assign),
            ("playlist_id", Assign),
            ("playlist_title", Assign),
            ("playlist_uploader", Assign),
            ("playlist_uploader_id", Assign),
            ("playlist_channel", Assign),
            ("playlist_channel_id", Assign),
            ("playlist_webpage_url", Assign),
            ("n_entries", Take),
            ("webpage_url", Take),
            ("webpage_url_basename", Take),
            ("webpage_url_domain", Take),
            ("playlist_index", Take),
            ("__last_playlist_index", Take),
            ("extractor", Take),
            ("extractor_key", Take),
            ("playlist_autonumber", Take),
        ] {
            replay.set(key, always);
        }
    } else {
        for key in [
            "webpage_url",
            "original_url",
            "webpage_url_basename",
            "webpage_url_domain",
            "extractor",
            "extractor_key",
        ] {
            replay.take(key);
        }
        // `process_video_result`: not part of a playlist.
        replay.assign("playlist");
        replay.assign("playlist_index");
    }
    // `process_video_result`: `_sanitize_thumbnails`, then `thumbnail`
    // and `_fill_common_fields`.
    replay.take("thumbnails");
    replay.take("thumbnail");
    replay.assign("display_id");
    replay.assign("fulltitle");
    for key in [
        "duration_string",
        "upload_date",
        "release_date",
        "modified_date",
    ] {
        replay.take(key);
    }
    replay.assign("release_year");
    replay.take("live_status");
    replay.take("is_live");
    replay.take("was_live");
    replay.assign("requested_subtitles");
    replay.assign("_has_drm");
    if untyped {
        replay.format_steps();
    }
    // The first `prepare_outtmpl` sets `epoch` before format selection.
    replay.take("epoch");
    // `-J`: `requested_downloads`, each without the keys whose values equal
    // the info dict's ("Remove copied info"); the selected format's keys are
    // merged afterwards (`info_dict.update(best_format)`), so the dict still
    // has the extractor's values (`None`) for them.
    if let Some(Value::Array(downloads)) = replay.ours.remove("requested_downloads") {
        let before: Map<String, Value> = replay
            .out
            .iter()
            .map(|(key, value)| {
                let value = if selected.contains(key) {
                    Value::Null
                } else {
                    value.clone()
                };
                (key.clone(), value)
            })
            .collect();
        let downloads = downloads
            .into_iter()
            .map(|download| match download {
                Value::Object(download) => Value::Object(
                    download
                        .into_iter()
                        .filter(|(key, value)| before.get(key).unwrap_or(&Value::Null) != value)
                        .collect(),
                ),
                other => other,
            })
            .collect();
        replay
            .out
            .insert("requested_downloads".into(), Value::Array(downloads));
    }
    // `info_dict.update(format)` (or `_merge`'s dict, whose keys are all
    // assigned): the format's values, including its `None`s.
    for key in &selected {
        match selected_format.get(key) {
            Some(value) if !replay.ours.contains_key(key) => {
                replay.out.insert(key.clone(), value.clone());
            }
            _ if merged => replay.assign(key),
            _ => replay.take(key),
        }
    }
    for key in [
        "_filename",
        "filename",
        "__postprocessors",
        "__real_download",
        "__files_to_merge",
        "__finaldir",
        "filepath",
        "__write_download_archive",
        "__last_playlist_index",
        "_type",
        "_version",
    ] {
        replay.take(key);
    }
    replay.finish()
}

/// A generic webpage playlist dict (`-J`) in upstream order.
pub(crate) fn playlist(mut map: Map<String, Value>) -> Map<String, Value> {
    let Some(Value::String(shape)) = map.remove(INFO_SHAPE) else {
        return map;
    };
    if shape != "html5-playlist" {
        return map;
    }
    let mut replay = Replay::new(map);
    // `playlist_result(entries, **info_dict)` of GenericIE's page fields.
    for (key, always) in [
        ("id", Take),
        ("title", Take),
        ("timestamp", Assign),
        ("description", Assign),
        ("thumbnail", Assign),
        ("age_limit", Assign),
        ("_type", Take),
        ("entries", Take),
    ] {
        replay.set(key, always);
    }
    let late = [
        "webpage_url",
        "original_url",
        "webpage_url_basename",
        "webpage_url_domain",
        "extractor",
        "extractor_key",
        "upload_date",
        "release_year",
        "playlist_count",
        "requested_entries",
        "epoch",
        "_version",
    ];
    replay.rest_except(&|key| late.contains(&key));
    for key in [
        "webpage_url",
        "original_url",
        "webpage_url_basename",
        "webpage_url_domain",
        "extractor",
        "extractor_key",
        "upload_date",
    ] {
        replay.take(key);
    }
    // `_fill_common_fields(final=False)`.
    replay.assign("release_year");
    for key in ["playlist_count", "requested_entries", "epoch"] {
        replay.take(key);
    }
    // Upstream's `-J` also prints the playlist's (empty) files to move.
    replay.set("__files_to_move", Dict);
    replay.take("_version");
    replay.finish()
}
