//! Upstream `--bidi-workaround`: messages pass through `bidiv` (else
//! `fribidi -c UTF-8`), whose output is read back from a pseudo-terminal so
//! it is line-buffered (`YoutubeDL._bidi_workaround`).
use std::sync::Mutex;

#[cfg(unix)]
struct Channel {
    stdin: std::process::ChildStdin,
    output: std::io::BufReader<std::fs::File>,
    _process: std::process::Child,
}

#[cfg(unix)]
static CHANNEL: Mutex<Option<Channel>> = Mutex::new(None);
#[cfg(not(unix))]
static CHANNEL: Mutex<Option<()>> = Mutex::new(None);

/// Start the external program. `Err` is upstream's warning for a missing
/// executable; other failures are errors.
#[cfg(unix)]
pub fn start() -> Result<(), StartError> {
    use std::process::{Command, Stdio};
    let (master, slave) = pty::open().map_err(StartError::Other)?;
    let width = columns();
    let spawn = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .args(["-w", &width.to_string()])
            .stdin(Stdio::piped())
            .stdout(slave.try_clone()?)
            .stderr(Stdio::inherit())
            .spawn()
    };
    let process = match spawn("bidiv", &[]) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            spawn("fribidi", &["-c", "UTF-8"])
        }
        result => result,
    }
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            StartError::NotFound
        } else {
            StartError::Other(error)
        }
    })?;
    drop(slave);
    let mut process = process;
    let stdin = process.stdin.take().expect("piped stdin");
    *CHANNEL.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(Channel {
        stdin,
        output: std::io::BufReader::new(master),
        _process: process,
    });
    Ok(())
}

/// Without `pty`, upstream's `import pty` fails before any program runs.
#[cfg(not(unix))]
pub fn start() -> Result<(), StartError> {
    Err(StartError::NotFound)
}

#[derive(Debug)]
pub enum StartError {
    NotFound,
    #[cfg_attr(not(unix), allow(dead_code))]
    Other(std::io::Error),
}

/// `_bidi_workaround(message)`: the program's lines for the message's lines
/// (as the terminal writes them, `\r\n`), without the final line break.
pub fn apply(message: &str) -> String {
    #[cfg(unix)]
    {
        use std::io::{BufRead, Write};
        let mut channel = CHANNEL.lock().unwrap_or_else(|poison| poison.into_inner());
        let Some(channel) = channel.as_mut() else {
            return message.to_owned();
        };
        let lines = message.matches('\n').count() + 1;
        if channel
            .stdin
            .write_all(format!("{message}\n").as_bytes())
            .and_then(|()| channel.stdin.flush())
            .is_err()
        {
            return message.to_owned();
        }
        let mut result = Vec::new();
        for _ in 0..lines {
            if channel.output.read_until(b'\n', &mut result).unwrap_or(0) == 0 {
                break;
            }
        }
        let mut result = String::from_utf8_lossy(&result).into_owned();
        if result.ends_with('\n') {
            result.pop();
        }
        result
    }
    #[cfg(not(unix))]
    {
        let _ = &CHANNEL;
        message.to_owned()
    }
}

/// Whether messages are being rewritten.
pub fn active() -> bool {
    CHANNEL
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .is_some()
}

/// `shutil.get_terminal_size().columns`: `COLUMNS`, else stdout's terminal
/// width, else 80.
#[cfg(unix)]
fn columns() -> u16 {
    if let Some(columns) = std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|columns| *columns > 0)
    {
        return columns;
    }
    pty::stdout_columns()
        .filter(|columns| *columns > 0)
        .unwrap_or(80)
}

#[cfg(unix)]
mod pty {
    use std::ffi::{CStr, c_char, c_int, c_ulong};
    use std::fs::File;
    use std::os::fd::FromRawFd;

    unsafe extern "C" {
        fn posix_openpt(flags: c_int) -> c_int;
        fn grantpt(fd: c_int) -> c_int;
        fn unlockpt(fd: c_int) -> c_int;
        fn ptsname(fd: c_int) -> *mut c_char;
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    }

    const O_RDWR: c_int = 2;
    #[cfg(target_os = "macos")]
    const O_NOCTTY: c_int = 0x20000;
    #[cfg(not(target_os = "macos"))]
    const O_NOCTTY: c_int = 0o400;
    #[cfg(target_os = "macos")]
    const TIOCGWINSZ: c_ulong = 0x4008_7468;
    #[cfg(not(target_os = "macos"))]
    const TIOCGWINSZ: c_ulong = 0x5413;

    /// `pty.openpty()`: the master and the opened slave.
    pub fn open() -> std::io::Result<(File, File)> {
        // SAFETY: plain libc calls on a descriptor this function owns;
        // `ptsname`'s static buffer is copied before any other call.
        unsafe {
            let master = posix_openpt(O_RDWR | O_NOCTTY);
            if master < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let master = File::from_raw_fd(master);
            use std::os::fd::AsRawFd;
            let fd = master.as_raw_fd();
            if grantpt(fd) != 0 || unlockpt(fd) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let name = ptsname(fd);
            if name.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let path = CStr::from_ptr(name).to_string_lossy().into_owned();
            let slave = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)?;
            Ok((master, slave))
        }
    }

    #[repr(C)]
    struct Winsize {
        rows: u16,
        columns: u16,
        x: u16,
        y: u16,
    }

    pub fn stdout_columns() -> Option<u16> {
        let mut size = Winsize {
            rows: 0,
            columns: 0,
            x: 0,
            y: 0,
        };
        // SAFETY: `size` is a valid `winsize` out-parameter for the call.
        (unsafe { ioctl(1, TIOCGWINSZ, &mut size as *mut Winsize) } == 0).then_some(size.columns)
    }
}
