//! Canonical paths as upstream shows and passes them. Upstream uses
//! `os.path.abspath`/`realpath`, which never produce Windows' verbatim
//! `\\?\` form that `std::fs::canonicalize` returns; FFmpeg arguments and
//! messages keep the plain form whenever it names the same file.
use std::path::{Path, PathBuf};

pub(crate) trait Canonical {
    /// `canonicalize`, without the verbatim prefix where it is not needed.
    fn canonical(&self) -> std::io::Result<PathBuf>;
}

impl Canonical for Path {
    fn canonical(&self) -> std::io::Result<PathBuf> {
        self.canonicalize().map(simplified)
    }
}

/// `\\?\C:\dir` as `C:\dir` and `\\?\UNC\server\share` as
/// `\\server\share`, unless the plain form would mean something else (a
/// path too long for the classic API, device names, components ending in
/// a dot or space, or `/`).
fn simplified(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let text = path.to_string_lossy();
    let plain = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
        && rest.as_bytes()[0].is_ascii_alphabetic()
    {
        rest.to_owned()
    } else {
        return path;
    };
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let unsafe_component = plain.split('\\').skip(1).any(|component| {
        let stem = component.split('.').next().unwrap_or_default().trim_end();
        component.ends_with(['.', ' '])
            || component.contains('/')
            || RESERVED.iter().any(|name| name.eq_ignore_ascii_case(stem))
    });
    if plain.len() >= 260 || unsafe_component || text.contains('\u{fffd}') {
        return path;
    }
    PathBuf::from(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_windows_paths_are_simplified_when_equivalent() {
        let cases: &[(&str, &str)] = &[
            (r"\\?\C:\Users\me\v.mp4", r"C:\Users\me\v.mp4"),
            (r"\\?\UNC\server\share\v.mp4", r"\\server\share\v.mp4"),
            (r"\\?\C:\dir\CON.txt", r"\\?\C:\dir\CON.txt"),
            (r"\\?\C:\dir\trailing.", r"\\?\C:\dir\trailing."),
            (r"\\?\Volume{1234}\v.mp4", r"\\?\Volume{1234}\v.mp4"),
            ("/tmp/v.mp4", "/tmp/v.mp4"),
        ];
        for (input, expected) in cases {
            let expected = if cfg!(windows) { *expected } else { *input };
            assert_eq!(simplified(PathBuf::from(input)), PathBuf::from(expected));
        }
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(simplified(PathBuf::from(&long)), PathBuf::from(&long));
    }
}
