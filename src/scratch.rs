//! Names for this port's private temporary files and directories beside
//! outputs (`.yt-dlp-rs-KIND-HOST-pidPID-RANDOM`). They carry the host and
//! process that created them, so ones a killed run left behind (SIGKILL, a
//! crash or a power loss skip their cleanup) are removed the next time this
//! port writes to the directory. Entries of live processes, other hosts
//! (containers sharing a volume) and untagged names are never touched.
//!
//! A postprocessing workspace may hold the only copy of a finished download
//! (`source.EXT`, moved there before the processors ran). It is moved back
//! to its name instead, where a rerun processes it as already downloaded.
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

/// The marker naming a postprocessing workspace's destination file.
pub(crate) const DESTINATION_MARKER: &str = "destination";

static SWEPT: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// FNV-1a of the host name: the same in every process on this host.
static HOST: LazyLock<u32> = LazyLock::new(|| {
    host_name().bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
});

static TAGGED: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^\.yt-dlp-rs-(?:[a-z]+-)*([0-9a-f]{8})-pid([0-9]+)-").unwrap()
});

#[cfg(unix)]
fn host_name() -> String {
    unsafe extern "C" {
        fn gethostname(name: *mut std::ffi::c_char, length: usize) -> i32;
    }
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is writable for its length; the name may not be
    // terminated when truncated, so the length is bounded below.
    if unsafe { gethostname(buffer.as_mut_ptr().cast(), buffer.len() - 1) } != 0 {
        return String::new();
    }
    let end = buffer.iter().position(|&byte| byte == 0).unwrap_or(0);
    String::from_utf8_lossy(&buffer[..end]).into_owned()
}

#[cfg(not(unix))]
fn host_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

/// `base` (such as `.yt-dlp-rs-merge-`) tagged with this host and process,
/// after removing what dead processes left in `directory`.
pub(crate) fn prefix(base: &str, directory: &Path) -> String {
    sweep(directory);
    format!("{base}{:08x}-pid{}-", *HOST, std::process::id())
}

/// Remove (or recover) entries of dead processes in `directory`, once per
/// directory and process.
pub(crate) fn sweep(directory: &Path) {
    {
        let Ok(mut swept) = SWEPT.lock() else {
            return;
        };
        if !swept
            .get_or_insert_with(HashSet::new)
            .insert(directory.to_owned())
        {
            return;
        }
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !abandoned(name) {
            continue;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            // A workspace whose finished download could not be recovered
            // is kept rather than deleting the only copy.
            if !name.starts_with(".yt-dlp-rs-process-") || recover(&path, directory) {
                let _ = fs::remove_dir_all(&path);
            }
        } else {
            let _ = fs::remove_file(&path);
        }
    }
}

/// Whether `name` is a tagged entry of a dead process on this host.
fn abandoned(name: &str) -> bool {
    let Some(found) = TAGGED.captures(name) else {
        return false;
    };
    let (Ok(host), Ok(pid)) = (u32::from_str_radix(&found[1], 16), found[2].parse::<u32>()) else {
        return false;
    };
    host == *HOST && pid != std::process::id() && !alive(pid)
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    const ESRCH: i32 = 3;
    let Ok(pid) = i32::try_from(pid) else {
        return true;
    };
    // SAFETY: signal 0 only checks that the process exists.
    if unsafe { kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    type Handle = *mut std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const ERROR_INVALID_PARAMETER: i32 = 87;
    const STILL_ACTIVE: u32 = 259;
    // SAFETY: plain Win32 calls; the handle is closed before returning.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            // No such process; access denied means it exists.
            return std::io::Error::last_os_error().raw_os_error() != Some(ERROR_INVALID_PARAMETER);
        }
        let mut code = 0;
        let running = GetExitCodeProcess(process, &mut code) == 0 || code == STILL_ACTIVE;
        CloseHandle(process);
        running
    }
}

#[cfg(not(any(unix, windows)))]
fn alive(_: u32) -> bool {
    // Without a liveness check, nothing is removed.
    true
}

/// Move a dead run's finished, unprocessed download back to its name;
/// whether the workspace may be removed (nothing finished to recover, the
/// destination already exists, or the download was moved).
fn recover(workspace: &Path, directory: &Path) -> bool {
    let Ok(name) = fs::read_to_string(workspace.join(DESTINATION_MARKER)) else {
        return true;
    };
    let name = Path::new(name.trim());
    // Only a plain file name in the same directory.
    if name.components().count() != 1 || name.file_name().is_none() {
        return true;
    }
    let destination = directory.join(name);
    if fs::symlink_metadata(&destination).is_ok() {
        return true;
    }
    let Ok(entries) = fs::read_dir(workspace) else {
        return false;
    };
    let sources: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                // The workspace input `source.EXT` (`source.flv` too), not a
                // format file (`source.f137.mp4`) or a postprocessor's
                // intermediate (`source.temp.mp4`).
                .and_then(|name| name.strip_prefix("source."))
                .is_some_and(|ext| !ext.is_empty() && !ext.contains('.'))
        })
        .collect();
    match sources.as_slice() {
        [] => true,
        [source] if fs::rename(source, &destination).is_ok() => {
            crate::screen::warning(&format!(
                "Recovered {} from an interrupted run; it has not been postprocessed",
                destination.display()
            ));
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only Unix has the liveness check the sweep needs.
    #[cfg(unix)]
    #[test]
    fn dead_processes_entries_are_removed_and_a_finished_download_recovered() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        // A PID that cannot be running (above any pid_max).
        let dead = format!("{:08x}-pid{}-", *HOST, i32::MAX as u32 - 1);
        let other_host = format!("{:08x}-pid{}-", HOST.wrapping_add(1), i32::MAX as u32 - 1);
        let live = format!("{:08x}-pid{}-", *HOST, std::process::id());
        fs::write(path.join(format!(".yt-dlp-rs-{dead}abc.part")), "x").unwrap();
        fs::create_dir(path.join(format!(".yt-dlp-rs-merge-{dead}abc"))).unwrap();
        fs::write(path.join(format!(".yt-dlp-rs-{other_host}abc.part")), "x").unwrap();
        fs::write(path.join(format!(".yt-dlp-rs-{live}abc.part")), "x").unwrap();
        fs::write(path.join(".yt-dlp-rs-untagged.part"), "x").unwrap();
        let workspace = path.join(format!(".yt-dlp-rs-process-{dead}abc"));
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join(DESTINATION_MARKER), "video.mp4").unwrap();
        fs::write(workspace.join("source.mp4"), "media").unwrap();
        // An escaping marker is ignored.
        let escaping = path.join(format!(".yt-dlp-rs-process-{dead}def"));
        fs::create_dir(&escaping).unwrap();
        fs::write(escaping.join(DESTINATION_MARKER), "../escaped.mp4").unwrap();
        fs::write(escaping.join("source.mp4"), "media").unwrap();
        // An "f"-initial extension, beside an interrupted fixup's output.
        let fixup = path.join(format!(".yt-dlp-rs-process-{dead}ghi"));
        fs::create_dir(&fixup).unwrap();
        fs::write(fixup.join(DESTINATION_MARKER), "audio.flac").unwrap();
        fs::write(fixup.join("source.flac"), "audio").unwrap();
        fs::write(fixup.join("source.temp.flac"), "partial").unwrap();
        // An ambiguous workspace is kept rather than deleted.
        let ambiguous_name = format!(".yt-dlp-rs-process-{dead}jkl");
        let ambiguous = path.join(&ambiguous_name);
        fs::create_dir(&ambiguous).unwrap();
        fs::write(ambiguous.join(DESTINATION_MARKER), "other.mkv").unwrap();
        fs::write(ambiguous.join("source.mp4"), "a").unwrap();
        fs::write(ambiguous.join("source.webm"), "b").unwrap();
        sweep(path);
        let mut names: Vec<String> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut expected = vec![
            format!(".yt-dlp-rs-{other_host}abc.part"),
            format!(".yt-dlp-rs-{live}abc.part"),
            ".yt-dlp-rs-untagged.part".to_owned(),
            "video.mp4".to_owned(),
            "audio.flac".to_owned(),
            ambiguous_name,
        ];
        expected.sort();
        assert_eq!(names, expected);
        assert_eq!(fs::read_to_string(path.join("video.mp4")).unwrap(), "media");
        assert_eq!(
            fs::read_to_string(path.join("audio.flac")).unwrap(),
            "audio"
        );
        assert!(
            !directory
                .path()
                .parent()
                .unwrap()
                .join("escaped.mp4")
                .exists()
        );
        // Each directory is swept once per process.
        fs::write(path.join(format!(".yt-dlp-rs-{dead}later.part")), "x").unwrap();
        sweep(path);
        assert!(path.join(format!(".yt-dlp-rs-{dead}later.part")).exists());
    }

    #[test]
    fn prefixes_carry_this_host_and_process() {
        let directory = tempfile::tempdir().unwrap();
        let prefix = prefix(".yt-dlp-rs-merge-", directory.path());
        assert!(TAGGED.is_match(&format!("{prefix}x")), "{prefix}");
        assert!(!abandoned(&format!("{prefix}x")));
    }
}
