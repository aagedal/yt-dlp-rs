//! Port extension `--growing-file`: a live download is written by FFmpeg as
//! one fragmented QuickTime file at its final name, so editors can import
//! it while it is recorded. FFmpeg's `-progress` reports how much media the
//! file holds. On macOS the file carries Blackmagic's recording attribute
//! while it grows: DaVinci Resolve reads it at import and treats the clip as
//! a growing (REC) clip, refreshing it every few seconds. The attribute is
//! removed when the recording ends. See Aagedal Media Converter's
//! `docs/growing-file-research/FINDINGS.md` (§0.3) for how it was found.
use std::{
    ffi::OsString,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// The growing file's extension: QuickTime, which Resolve and other
/// editors take as a growing clip, unless a codec FFmpeg's MOV muxer refuses
/// is selected ("av1 only supported in MP4 and AVIF"; VP9, Opus and FLAC
/// likewise); those are written as fragmented MP4 instead, the container
/// Resolve expects AV1 in.
pub(crate) fn ext(info: &crate::VideoInfo) -> &'static str {
    let mut codecs = Vec::new();
    if info.requested_formats.is_empty() {
        codecs.extend([&info.vcodec, &info.acodec]);
    }
    for format in &info.requested_formats {
        codecs.extend([&format.vcodec, &format.acodec]);
    }
    if codecs.into_iter().flatten().any(|codec| mp4_only(codec)) {
        "mp4"
    } else {
        "mov"
    }
}

/// Whether FFmpeg writes `codec` (an RFC 6381 string or codec name) in MP4
/// but not in MOV.
fn mp4_only(codec: &str) -> bool {
    let name = codec
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "av01" | "av1" | "vp09" | "vp9" | "opus" | "flac"
    )
}

/// Blackmagic's extended attribute, `{"r":1, "uuid":"HEX"}` while recording.
#[cfg(target_os = "macos")]
const RECORDING_ATTRIBUTE: &str = "com.blackmagicdesign.metadata:recording";

/// FFmpeg output options before the output file: progress on stdout instead
/// of the stats line, a timecode track, and fragments at each video keyframe
/// (and at least every second), flushed as they are written, in the `ext`
/// muxer (`mov` or `mp4`, see [`ext`]).
pub(crate) fn output_args(timecode: &str, ext: &str) -> Vec<OsString> {
    [
        "-nostats",
        "-progress",
        "pipe:1",
        "-timecode",
        timecode,
        "-movflags",
        "+frag_keyframe",
        "-frag_duration",
        "1000000",
        "-flush_packets",
        "1",
        "-f",
        ext,
    ]
    .map(OsString::from)
    .into()
}

/// The recording's start as an `HH:MM:SS:00` time-of-day timecode in local
/// time (`unix` seconds; UTC where the local offset is unavailable).
pub(crate) fn timecode(unix: i64) -> String {
    let seconds = crate::dates::local_seconds_of_day(unix);
    format!(
        "{:02}:{:02}:{:02}:00",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

/// The current time in Unix seconds.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// What FFmpeg's `-progress` blocks reported so far.
#[derive(Debug, Default)]
pub(crate) struct Progress {
    /// Completed `-progress` blocks; changes when a new report arrives.
    reports: AtomicU64,
    /// The media written (`out_time_us`), in milliseconds.
    recorded_ms: AtomicU64,
    /// The output's size (`total_size`).
    bytes: AtomicU64,
}

impl Progress {
    /// The report count, recorded milliseconds and bytes.
    pub(crate) fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.reports.load(Ordering::SeqCst),
            self.recorded_ms.load(Ordering::SeqCst),
            self.bytes.load(Ordering::SeqCst),
        )
    }

    /// One `key=value` line of a `-progress` block (`N/A` and negative
    /// values at the start are ignored).
    fn line(&self, line: &str) {
        let Some((key, value)) = line.trim_end().split_once('=') else {
            return;
        };
        match key {
            "out_time_us" => {
                if let Ok(micros) = value.parse::<u64>() {
                    self.recorded_ms.store(micros / 1000, Ordering::SeqCst);
                }
            }
            "total_size" => {
                if let Ok(bytes) = value.parse::<u64>() {
                    self.bytes.store(bytes, Ordering::SeqCst);
                }
            }
            "progress" => {
                self.reports.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
    }

    /// Read FFmpeg's `-progress pipe:1` output until it closes.
    pub(crate) fn follow(
        self: Arc<Self>,
        output: impl Read + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                self.line(&line);
            }
        })
    }
}

/// The recording attribute on a growing file, removed when dropped.
pub(crate) struct RecordingTag {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    path: PathBuf,
}

impl RecordingTag {
    /// Tag `path` (macOS only; elsewhere nothing is written). A failure is
    /// a warning: the file still grows, Resolve just refreshes it slower.
    pub(crate) fn set(path: &Path) -> Self {
        #[cfg(target_os = "macos")]
        {
            let value = format!("{{\"r\":1, \"uuid\":\"{}\"}}", uuid());
            if let Err(error) = macos::set(path, RECORDING_ATTRIBUTE, value.as_bytes()) {
                crate::screen::warning(&format!(
                    "Unable to mark {} as a growing recording: {error}",
                    path.display()
                ));
            }
        }
        Self {
            path: path.to_owned(),
        }
    }
}

impl Drop for RecordingTag {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if let Err(error) = macos::remove(&self.path, RECORDING_ATTRIBUTE)
            && error.raw_os_error() != Some(macos::ENOATTR)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            crate::screen::warning(&format!(
                "Unable to clear the growing recording mark of {}: {error}",
                self.path.display()
            ));
        }
    }
}

/// 32 uppercase hexadecimal digits from the system's random source.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn uuid() -> String {
    let mut bytes = [0_u8; 16];
    let random = std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .is_ok();
    if !random {
        // A process- and time-specific value is unique enough here.
        use std::hash::{BuildHasher, Hasher};
        let state = std::collections::hash_map::RandomState::new();
        for (index, chunk) in bytes.chunks_mut(8).enumerate() {
            let mut hasher = state.build_hasher();
            hasher.write_u128(
                std::time::UNIX_EPOCH
                    .elapsed()
                    .map_or(0, |time| time.as_nanos()),
            );
            hasher.write_usize(index);
            chunk.copy_from_slice(&hasher.finish().to_be_bytes());
        }
    }
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        ffi::{CString, c_char, c_int, c_void},
        os::unix::ffi::OsStrExt,
        path::Path,
    };

    /// `ENOATTR` on macOS.
    pub(super) const ENOATTR: i32 = 93;

    unsafe extern "C" {
        fn setxattr(
            path: *const c_char,
            name: *const c_char,
            value: *const c_void,
            size: usize,
            position: u32,
            options: c_int,
        ) -> c_int;
        fn removexattr(path: *const c_char, name: *const c_char, options: c_int) -> c_int;
    }

    fn strings(path: &Path, name: &str) -> std::io::Result<(CString, CString)> {
        Ok((
            CString::new(path.as_os_str().as_bytes())?,
            CString::new(name)?,
        ))
    }

    pub(super) fn set(path: &Path, name: &str, value: &[u8]) -> std::io::Result<()> {
        let (path, name) = strings(path, name)?;
        // SAFETY: valid NUL-terminated strings and a buffer of `value.len()` bytes.
        let result = unsafe {
            setxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    pub(super) fn remove(path: &Path, name: &str) -> std::io::Result<()> {
        let (path, name) = strings(path, name)?;
        // SAFETY: valid NUL-terminated strings.
        let result = unsafe { removexattr(path.as_ptr(), name.as_ptr(), 0) };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    #[cfg(test)]
    pub(super) fn get(path: &Path, name: &str) -> Option<Vec<u8>> {
        unsafe extern "C" {
            fn getxattr(
                path: *const c_char,
                name: *const c_char,
                value: *mut c_void,
                size: usize,
                position: u32,
                options: c_int,
            ) -> isize;
        }
        let (path, name) = strings(path, name).ok()?;
        let mut buffer = vec![0_u8; 256];
        // SAFETY: valid NUL-terminated strings and a writable buffer.
        let size = unsafe {
            getxattr(
                path.as_ptr(),
                name.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                0,
                0,
            )
        };
        (size >= 0).then(|| {
            buffer.truncate(size as usize);
            buffer
        })
    }
}

/// Named pipes feeding FFmpeg one input each (`--live-from-start` streams
/// downloaded by the native fragment downloader).
#[cfg(unix)]
pub(crate) mod fifo {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, path::Path};

    #[cfg(any(target_os = "linux", target_os = "android"))]
    type Mode = u32;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    type Mode = u16;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const O_NONBLOCK: i32 = 0o4000;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const O_NONBLOCK: i32 = 0x0004;

    unsafe extern "C" {
        fn mkfifo(path: *const std::ffi::c_char, mode: Mode) -> std::ffi::c_int;
    }

    pub(crate) fn create(path: &Path) -> std::io::Result<()> {
        let path = CString::new(path.as_os_str().as_bytes())?;
        // SAFETY: a valid NUL-terminated path.
        if unsafe { mkfifo(path.as_ptr(), 0o600) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    /// Open and close the reading end without blocking, which releases a
    /// writer waiting in `open` after FFmpeg has gone (its writes then fail).
    pub(crate) fn release(path: &Path) {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_blocks_report_recorded_time_and_size() {
        let progress = Progress::default();
        for line in [
            "out_time_us=N/A",
            "total_size=N/A",
            "progress=continue",
            "out_time_us=12021333",
            "total_size=1290646",
            "progress=continue\n",
        ] {
            progress.line(line);
        }
        assert_eq!(progress.snapshot(), (2, 12021, 1290646));
        // A negative start time leaves the previous value.
        progress.line("out_time_us=-23220");
        assert_eq!(progress.snapshot().1, 12021);
    }

    #[test]
    fn output_args_write_fragmented_quicktime_with_a_timecode() {
        let args: Vec<String> = output_args("12:34:56:00", "mov")
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect();
        assert_eq!(
            args.join(" "),
            "-nostats -progress pipe:1 -timecode 12:34:56:00 -movflags +frag_keyframe \
             -frag_duration 1000000 -flush_packets 1 -f mov"
        );
    }

    #[test]
    fn codecs_mov_cannot_hold_are_written_as_mp4() {
        for codec in [
            "av01.0.08M.08",
            "AV1",
            "vp09.00.40.08",
            "vp9",
            "opus",
            "flac",
        ] {
            assert!(mp4_only(codec), "{codec}");
        }
        for codec in [
            "avc1.64001F",
            "h264",
            "hvc1.1.6.L93.B0",
            "mp4a.40.2",
            "none",
            "",
        ] {
            assert!(!mp4_only(codec), "{codec}");
        }
    }

    #[test]
    fn uuids_are_32_uppercase_hex_digits() {
        let (first, second) = (uuid(), uuid());
        assert_eq!(first.len(), 32);
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        );
        assert_ne!(first, second);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_recording_tag_is_removed_when_dropped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("live.mov");
        std::fs::write(&path, b"").unwrap();
        let tag = RecordingTag::set(&path);
        let value = String::from_utf8(macos::get(&path, RECORDING_ATTRIBUTE).unwrap()).unwrap();
        assert!(value.starts_with("{\"r\":1, \"uuid\":\""), "{value}");
        assert!(
            value.ends_with("\"}") && value.len() == 16 + 32 + 2,
            "{value}"
        );
        // FFmpeg's truncating open keeps the attribute.
        std::fs::File::create(&path).unwrap();
        assert!(macos::get(&path, RECORDING_ATTRIBUTE).is_some());
        drop(tag);
        assert_eq!(macos::get(&path, RECORDING_ATTRIBUTE), None);
    }
}
