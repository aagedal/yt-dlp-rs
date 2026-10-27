//! Upstream `yt_dlp/cache.py`: JSON files at `ROOT/SECTION/KEY.json`, each
//! `{"yt-dlp_version": VERSION, "data": DATA}` as upstream writes them.
//!
//! The default root is `${XDG_CACHE_HOME:-~/.cache}/yt-dlp-rs`, this port's
//! own directory (like its `yt-dlp-rs` configuration files), so that
//! `--rm-cache-dir` never deletes upstream's cache and upstream never reads
//! entries this port wrote. The file format is upstream's: `--cache-dir`
//! pointed at an upstream cache directory reads and writes upstream entries.
//! Entries record [`UPSTREAM_VERSION`], the yt-dlp release this port tracks,
//! because readers compare it with upstream release versions (`min_ver`).
use serde_json::{Map, Value};
use std::path::PathBuf;
use std::sync::RwLock;

/// The pinned upstream `yt_dlp.version.__version__`.
pub const UPSTREAM_VERSION: &str = "2026.10.26";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Setting {
    /// `cachedir` unset: `${XDG_CACHE_HOME:-~/.cache}/yt-dlp-rs`.
    Default,
    /// `--cache-dir DIR`.
    Dir(String),
    /// `--no-cache-dir` (`cachedir=False`).
    Disabled,
}

// Unit tests never touch the user's cache unless they configure one.
static SETTING: RwLock<Setting> = RwLock::new(if cfg!(test) {
    Setting::Disabled
} else {
    Setting::Default
});

/// `--cache-dir DIR` (`Some`) or `--no-cache-dir` (`None`); the last of
/// them wins like upstream's shared `cachedir` destination.
pub fn configure(directory: Option<&str>) {
    *SETTING.write().unwrap() = match directory {
        Some(directory) => Setting::Dir(directory.to_owned()),
        None => Setting::Disabled,
    };
}

/// `Cache.enabled`.
pub fn enabled() -> bool {
    *SETTING.read().unwrap() != Setting::Disabled
}

/// `Cache._get_root_dir`.
pub fn root() -> PathBuf {
    let setting = SETTING.read().unwrap().clone();
    let root = match setting {
        Setting::Dir(directory) => directory,
        _ => {
            let root = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| "~/.cache".into());
            // `os.path.join(cache_root, 'yt-dlp')`.
            if root.is_empty() || root.ends_with('/') {
                format!("{root}{}", crate::config::PACKAGE_NAME)
            } else {
                format!("{root}/{}", crate::config::PACKAGE_NAME)
            }
        }
    };
    crate::batch::expand_path(&root)
}

/// `Cache._get_cache_fn`: the section must match `^[\w.-]+$`; the key is
/// percent-encoded (`quote(key, safe='')`) with `%` written as `,`.
fn cache_file(section: &str, key: &str) -> PathBuf {
    assert!(
        !section.is_empty()
            && section
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-'),
        "invalid section {section:?}"
    );
    let mut name = String::new();
    for byte in key.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b'~') {
            name.push(byte as char);
        } else {
            name.push_str(&format!(",{byte:02X}"));
        }
    }
    root().join(section).join(format!("{name}.json"))
}

/// `Cache.store(section, key, data)`: written atomically through a
/// temporary file; failures are warnings.
pub fn store(section: &str, key: &str, data: &Value) {
    if !enabled() {
        return;
    }
    let path = cache_file(section, key);
    let result = (|| -> std::io::Result<()> {
        let directory = path.parent().expect("section directory");
        std::fs::create_dir_all(directory)?;
        crate::screen::debug(&format!("Saving {section}.{key} to cache"));
        let mut envelope = Map::new();
        envelope.insert("yt-dlp_version".into(), UPSTREAM_VERSION.into());
        envelope.insert("data".into(), data.clone());
        let bytes = crate::info_json::to_python_json(&Value::Object(envelope))
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
        // Upstream's temporary file has `0o666 & ~umask` permissions.
        let mut temporary = tempfile::Builder::new()
            .prefix(&format!("{file_name}."))
            .suffix(".tmp")
            .make_in(directory, |path| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
            })?;
        std::io::Write::write_all(temporary.as_file_mut(), &bytes)?;
        temporary.persist(&path).map_err(|error| error.error)?;
        Ok(())
    })();
    if let Err(error) = result {
        // Upstream appends the Python traceback; this is the OS error.
        crate::screen::warning(&format!(
            "Writing cache to '{}' failed: {error}",
            path.display()
        ));
    }
}

/// `version_tuple(v)` (not lenient): `None` where Python raises `ValueError`.
fn version_tuple(version: &str) -> Option<Vec<i64>> {
    version
        .split(['-', '.'])
        .map(|part| part.trim().parse::<i64>().ok())
        .collect()
}

/// `Cache.load(section, key, min_ver=...)`: `None` for a missing,
/// disabled or outdated entry (and after a corrupt entry's warning).
pub fn load(section: &str, key: &str, min_ver: Option<&str>) -> Option<Value> {
    if !enabled() {
        return None;
    }
    let path = cache_file(section, key);
    let bytes = std::fs::read(&path).ok()?;
    crate::screen::debug(&format!("Loading {section}.{key} from cache"));
    let parsed = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| crate::json::from_str::<Value>(text).ok());
    let failed = || {
        let size = std::fs::metadata(&path)
            .map(|metadata| metadata.len().to_string())
            .unwrap_or_else(|error| error.to_string());
        crate::screen::warning(&format!(
            "Cache retrieval from {} failed ({size})",
            path.display()
        ));
        None
    };
    let Some(parsed) = parsed else {
        return failed();
    };
    // `_validate`: an entry without a version predates versioned entries.
    let version = parsed
        .get("yt-dlp_version")
        .filter(|version| match version {
            Value::Null | Value::Bool(false) => false,
            Value::String(text) => !text.is_empty(),
            _ => true,
        })
        .cloned();
    let (data, version) = match version {
        None => (parsed, "2022.08.19".to_owned()),
        Some(Value::String(version)) => match parsed.get("data") {
            Some(data) => (data.clone(), version),
            None => return failed(),
        },
        Some(_) => return failed(),
    };
    let Some(min_ver) = min_ver.filter(|min_ver| !min_ver.is_empty()) else {
        return Some(data);
    };
    match (version_tuple(&version), version_tuple(min_ver)) {
        (Some(found), Some(needed)) if found >= needed => Some(data),
        (Some(_), Some(_)) => {
            crate::screen::debug(&format!(
                "Discarding old cache from version {version} (needs {min_ver})"
            ));
            None
        }
        _ => failed(),
    }
}

/// `Cache.remove` (`--rm-cache-dir`): upstream's messages, refusing a
/// directory whose path contains neither `cache` nor `tmp`.
pub fn remove() -> crate::Result<()> {
    if !enabled() {
        crate::screen::screen(
            "Cache is disabled (Did you combine --no-cache-dir and --rm-cache-dir?)",
        );
        return Ok(());
    }
    let directory = root();
    let text = directory.to_string_lossy();
    if !text.contains("cache") && !text.contains("tmp") {
        return Err(crate::Error::Reported(format!(
            "Not removing directory {text} - this does not look like a cache dir"
        )));
    }
    let mut line = format!("Removing cache dir {text} .");
    if directory.exists() {
        line.push('.');
        std::fs::remove_dir_all(&directory).map_err(crate::Error::Io)?;
    }
    line.push('.');
    crate::screen::screen(&line);
    Ok(())
}

/// Tests share the process-wide setting.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn with_root<T>(run: impl FnOnce(&std::path::Path) -> T) -> T {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("cache");
        configure(Some(root.to_str().unwrap()));
        let result = run(&root);
        configure(None);
        result
    }

    #[test]
    fn stores_upstream_envelopes_and_loads_them() {
        with_root(|root| {
            store("youtube-sigfuncs", "abc-main-3", &json!([2, 0, 1]));
            let text =
                std::fs::read_to_string(root.join("youtube-sigfuncs/abc-main-3.json")).unwrap();
            assert_eq!(
                text,
                r#"{"yt-dlp_version": "2026.10.26", "data": [2, 0, 1]}"#
            );
            assert_eq!(
                load("youtube-sigfuncs", "abc-main-3", Some("2025.07.21")),
                Some(json!([2, 0, 1]))
            );
            assert_eq!(
                load("youtube-sigfuncs", "abc-main-3", Some("2027.01.01")),
                None
            );
            assert_eq!(load("youtube-sigfuncs", "missing", None), None);
            // `store(..., None)` clears an entry to JSON `null`.
            store("soundcloud", "client_id", &Value::Null);
            assert_eq!(load("soundcloud", "client_id", None), Some(Value::Null));
        });
    }

    #[test]
    fn keys_are_quoted_with_commas() {
        with_root(|root| {
            store("brightcove", "1_default é/?", &json!("pk"));
            assert!(
                root.join("brightcove/1_default,20,C3,A9,2F,3F.json")
                    .exists()
            );
        });
    }

    #[test]
    fn unversioned_and_corrupt_entries() {
        with_root(|root| {
            std::fs::create_dir_all(root.join("s")).unwrap();
            // Entries from before 2022.08.19 are the bare data.
            std::fs::write(root.join("s/old.json"), r#"{"a": 1}"#).unwrap();
            assert_eq!(load("s", "old", None), Some(json!({"a": 1})));
            assert_eq!(load("s", "old", Some("2022.08.20")), None);
            std::fs::write(root.join("s/bad.json"), "{").unwrap();
            assert_eq!(load("s", "bad", None), None);
            std::fs::write(root.join("s/nodata.json"), r#"{"yt-dlp_version": "1"}"#).unwrap();
            assert_eq!(load("s", "nodata", None), None);
        });
    }

    #[test]
    #[should_panic(expected = "invalid section")]
    fn sections_are_validated() {
        let _ = cache_file("bad/section", "key");
    }

    #[test]
    fn version_tuples_follow_python_int() {
        assert_eq!(version_tuple("2025.07.21"), Some(vec![2025, 7, 21]));
        assert_eq!(version_tuple("2025.07.21-dev"), None);
    }
}
