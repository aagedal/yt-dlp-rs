//! Shared helpers for integration tests (`mod support;`).
#![allow(dead_code)]
use std::path::{Path, PathBuf};

/// A fake executable `name` in `directory` that runs the POSIX shell
/// `script`, returning its path. On Unix the script itself is the program;
/// on Windows `name.exe` is a compiled shim (`tests/helpers/sh_shim.rs`)
/// running `name.sh` with Git for Windows' `sh.exe` (or
/// `YT_DLP_RS_TEST_SH`), so the same scripts stand in for FFmpeg and other
/// tools everywhere.
pub fn fake_tool(directory: &Path, name: &str, script: &str) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    #[cfg(windows)]
    {
        let path = directory.join(format!("{name}.exe"));
        std::fs::write(path.with_extension("sh"), script).unwrap();
        std::fs::write(
            path.with_extension("shell"),
            windows::shell().to_string_lossy().as_bytes(),
        )
        .unwrap();
        std::fs::copy(windows::shim(), &path).unwrap();
        path
    }
}

/// `text` with Windows path separators as `/`, for comparing output that
/// names files (a no-op elsewhere).
pub fn slashes(text: &str) -> String {
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.to_owned()
    }
}

/// Let [`stop`] reach a child started from `command`: on Windows it gets
/// its own process group, the target of a console Ctrl+Break.
pub fn stoppable(command: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP)
    }
    #[cfg(not(windows))]
    command
}

/// Ask a child to stop: SIGTERM, or on Windows Ctrl+Break to its process
/// group (see [`stoppable`]), which the CLI handles alike. SIGTERM rather
/// than SIGINT, since background test processes may inherit an ignored
/// SIGINT (and Windows cannot send Ctrl+C to one process group).
pub fn stop(child: &std::process::Child) {
    signal(child, "-TERM");
}

/// Interrupt a child: SIGINT, or Ctrl+Break on Windows (see [`stop`]).
pub fn interrupt(child: &std::process::Child) {
    signal(child, "-INT");
}

fn signal(child: &std::process::Child, _signal: &str) {
    #[cfg(unix)]
    {
        let status = std::process::Command::new("kill")
            .args([_signal, &child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32;
        }
        const CTRL_BREAK_EVENT: u32 = 1;
        // SAFETY: a plain Win32 call; the group is the child's own.
        let sent = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) };
        assert_ne!(sent, 0, "{}", std::io::Error::last_os_error());
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        path::{Path, PathBuf},
        process::Command,
        sync::OnceLock,
    };

    pub fn shim() -> &'static Path {
        static SHIM: OnceLock<PathBuf> = OnceLock::new();
        SHIM.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap().keep();
            let binary = directory.join("sh-shim.exe");
            let status = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/helpers/sh_shim.rs"))
                .args(["--edition=2024", "-Dwarnings", "-o"])
                .arg(&binary)
                .status()
                .unwrap();
            assert!(status.success(), "compiling tests/helpers/sh_shim.rs");
            binary
        })
    }

    /// `sh.exe` from `YT_DLP_RS_TEST_SH`, PATH or Git for Windows, preferring
    /// the copy in `usr\bin` beside the POSIX utilities.
    pub fn shell() -> &'static Path {
        static SHELL: OnceLock<PathBuf> = OnceLock::new();
        SHELL.get_or_init(|| {
            if let Some(shell) = std::env::var_os("YT_DLP_RS_TEST_SH") {
                return shell.into();
            }
            let program_files = std::env::var_os("ProgramFiles")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
            let on_path = std::env::var_os("PATH")
                .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
                .unwrap_or_default()
                .into_iter()
                .map(|directory| directory.join("sh.exe"));
            let found = on_path
                .chain([
                    program_files.join(r"Git\usr\bin\sh.exe"),
                    program_files.join(r"Git\bin\sh.exe"),
                ])
                .find(|candidate| candidate.is_file())
                .expect("a POSIX sh.exe (Git for Windows) or YT_DLP_RS_TEST_SH");
            // `Git\bin\sh.exe` -> `Git\usr\bin\sh.exe`.
            let usr = found
                .parent()
                .and_then(Path::parent)
                .map(|root| root.join(r"usr\bin\sh.exe"))
                .filter(|candidate| candidate.is_file());
            usr.unwrap_or(found)
        })
    }
}
