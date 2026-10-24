//! Python's `getpass.getpass(prompt)` as upstream uses it for the account
//! password (`-u` without `-p`) and two-factor codes (no `-2`): the prompt is
//! written to and the answer read from the controlling terminal with echo
//! turned off (through `stty`, as this port has no termios binding).
//! Without a terminal, like `fallback_getpass`, `Warning: Password input
//! may be echoed.` and the prompt go to stderr and the answer is read from
//! stdin; end of input is an error (Python's `EOFError`).
use std::io::{BufRead, Write};

/// Read a secret after `prompt`, without its line ending.
pub fn getpass(prompt: &str) -> std::io::Result<String> {
    #[cfg(unix)]
    if let Ok(tty) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    {
        return tty_getpass(tty, prompt);
    }
    let mut stderr = std::io::stderr();
    writeln!(stderr, "Warning: Password input may be echoed.")?;
    write!(stderr, "{prompt}")?;
    stderr.flush()?;
    read_line(&mut std::io::stdin().lock())
}

#[cfg(unix)]
fn tty_getpass(tty: std::fs::File, prompt: &str) -> std::io::Result<String> {
    let stty = |args: &[&str]| {
        std::process::Command::new("stty")
            .args(args)
            .stdin(tty.try_clone()?)
            .stderr(std::process::Stdio::null())
            .output()
    };
    let saved = stty(&["-g"])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    if saved.is_some() {
        let _ = stty(&["-echo"]);
    }
    let mut writer = tty.try_clone()?;
    write!(writer, "{prompt}")?;
    writer.flush()?;
    let answer = read_line(&mut std::io::BufReader::new(tty.try_clone()?));
    if let Some(saved) = saved {
        let _ = stty(&[saved.as_str()]);
    }
    writeln!(writer)?;
    answer
}

fn read_line(reader: &mut impl BufRead) -> std::io::Result<String> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "EOF when reading a line",
        ));
    }
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    #[test]
    fn answers_lose_their_line_ending_and_eof_is_an_error() {
        let mut input: &[u8] = b"s3cret\r\nnext\n";
        assert_eq!(super::read_line(&mut input).unwrap(), "s3cret");
        assert_eq!(super::read_line(&mut input).unwrap(), "next");
        assert!(super::read_line(&mut input).is_err());
    }
}
