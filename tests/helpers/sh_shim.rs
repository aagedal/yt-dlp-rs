//! Windows stand-in for a fake `#!/bin/sh` executable (see
//! `tests/support/mod.rs`): copied to `NAME.exe`, it runs the script
//! `NAME.sh` beside it with the POSIX shell named in `NAME.shell` (Git for
//! Windows' `sh.exe`), passing its arguments, standard streams and exit
//! code through. The shell and everything it starts end with the shim.
//! Console Ctrl+C/Ctrl+Break events are ignored (the shell runs in its own
//! process group), as Unix tests signal only the program under test.
use std::{path::PathBuf, process::Command};

fn main() {
    let exe = std::env::current_exe().expect("shim path");
    let shell = std::fs::read_to_string(exe.with_extension("shell")).expect("shell path file");
    let shell = PathBuf::from(shell.trim());
    let script = exe.with_extension("sh");
    let mut command = Command::new(&shell);
    // The shell's own utilities (`cat`, `sleep`, ...) live beside it; the
    // program under test may have cleared PATH.
    let mut path = vec![shell.parent().expect("shell directory").to_path_buf()];
    if let Some(inherited) = std::env::var_os("PATH") {
        path.extend(std::env::split_paths(&inherited));
    }
    command.env("PATH", std::env::join_paths(path).expect("PATH"));
    let script = script.to_string_lossy().replace('\\', "/");
    quote_args(&mut command, std::iter::once(script).chain(std::env::args().skip(1)));
    kill_on_exit();
    ignore_console_events(&mut command);
    let status = command.status().unwrap_or_else(|error| {
        eprintln!("sh_shim: cannot start {}: {error}", shell.display());
        std::process::exit(127)
    });
    std::process::exit(status.code().unwrap_or(1));
}

/// Every argument double-quoted (backslashes doubled only before a quote,
/// like the MSVC runtime reads them), so the MSYS runtime neither globs
/// `*`/`?` in URLs nor splits paths.
#[cfg(windows)]
fn quote_args(command: &mut Command, args: impl Iterator<Item = String>) {
    use std::os::windows::process::CommandExt;
    for arg in args {
        let mut quoted = String::from("\"");
        let mut backslashes = 0;
        for c in arg.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                    backslashes = 0;
                }
                _ => {
                    quoted.extend(std::iter::repeat_n('\\', backslashes));
                    backslashes = 0;
                }
            }
            if c != '\\' {
                quoted.push(c);
            }
        }
        quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
        quoted.push('"');
        command.raw_arg(quoted);
    }
}

#[cfg(not(windows))]
fn quote_args(command: &mut Command, args: impl Iterator<Item = String>) {
    command.args(args);
}

/// Put this process in a job that kills its members when the last handle
/// closes: when the program under test kills the shim, the shell goes too
/// instead of holding the shim's pipes open.
#[cfg(windows)]
fn kill_on_exit() {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io_counters: [u64; 6],
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: i32,
            information: *const c_void,
            length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
    }
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
    let limits = ExtendedLimits {
        basic: BasicLimits {
            limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: plain Win32 calls with a valid, correctly sized limits struct;
    // the job handle is deliberately kept open until the process exits.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return;
        }
        SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            (&raw const limits).cast(),
            std::mem::size_of::<ExtendedLimits>() as u32,
        );
        AssignProcessToJobObject(job, GetCurrentProcess());
    }
}

#[cfg(not(windows))]
fn kill_on_exit() {}

#[cfg(windows)]
fn ignore_console_events(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }
    extern "system" fn ignore(_: u32) -> i32 {
        1
    }
    // SAFETY: registers a handler that only returns TRUE.
    unsafe {
        SetConsoleCtrlHandler(Some(ignore), 1);
    }
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(windows))]
fn ignore_console_events(_: &mut Command) {}
