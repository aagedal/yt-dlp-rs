//! The minimal `_get_login_info(username_option, password_option,
//! netrc_machine)` Adobe Pass needs: the explicit options, else
//! `_get_netrc_login_info` from `--netrc-cmd` or the `--netrc`
//! (`--netrc-location`) file with CPython's `netrc` parser (3.11+). Kept
//! behind [`login_info`] so a general login subsystem can replace it.
use crate::{Error, Result};
use std::collections::HashMap;

/// The account options `_get_login_info` reads.
#[derive(Clone, Debug, Default)]
pub struct LoginOptions {
    pub username: Option<String>,
    pub password: Option<String>,
    /// `usenetrc` (`--netrc`).
    pub netrc: bool,
    pub netrc_location: Option<String>,
    pub netrc_cmd: Option<String>,
}

/// `_get_login_info`: `(username, password)` from the options, else netrc
/// (`[EXTRACTOR] ` prefixes its messages); `Failed to parse .netrc` warns
/// and gives no credentials.
pub(super) fn login_info(
    options: &LoginOptions,
    machine: &str,
    extractor: &str,
) -> Result<(Option<String>, Option<String>)> {
    if options.username.is_some() {
        return Ok((options.username.clone(), options.password.clone()));
    }
    // `Invalid netrc machine` (an `ExtractorError`, not a parse failure).
    const ALLOWED: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-_";
    if machine.starts_with(['-', '_']) || !machine.chars().all(|c| ALLOWED.contains(c)) {
        return Err(Error::Reported(format!(
            "[{extractor}] Invalid netrc machine: {}",
            crate::batch::py_repr(machine)
        )));
    }
    let content = if let Some(command) = &options.netrc_cmd {
        let command = command.replace("{}", machine);
        crate::screen::screen(&format!("[{extractor}] Executing command: {command}"));
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .stderr(std::process::Stdio::inherit())
            .output();
        match output {
            Ok(output) if output.status.success() => Ok((
                String::from_utf8_lossy(&output.stdout).into_owned(),
                "<string>".to_owned(),
            )),
            Ok(output) => Err(format!(
                "Command returned error code {}",
                output.status.code().unwrap_or(-1)
            )),
            Err(error) => Err(os_error(&error, None)),
        }
    } else if options.netrc {
        let location = options.netrc_location.as_deref().unwrap_or("~");
        let mut path = std::path::PathBuf::from(expand_user(location));
        if path.is_dir() {
            path = path.join(".netrc");
        }
        let name = path.to_string_lossy().into_owned();
        std::fs::read_to_string(&path)
            .map(|text| (text, name.clone()))
            .map_err(|error| os_error(&error, Some(&name)))
    } else {
        return Ok((None, None));
    };
    let info = content.and_then(|(text, file)| {
        parse(&text, &file)
            .map(|hosts| hosts.get(machine).or_else(|| hosts.get("default")).cloned())
    });
    match info {
        Err(error) => {
            crate::screen::warning(&format!("[{extractor}] Failed to parse .netrc: {error}"));
            Ok((None, None))
        }
        Ok(None) => {
            crate::screen::screen(&format!("[{extractor}] No authenticators for {machine}"));
            Ok((None, None))
        }
        Ok(Some((login, password))) => {
            crate::screen::extractor_debug(&format!("Using netrc for {machine} authentication"));
            Ok((Some(login), Some(password)))
        }
    }
}

/// `compat_expanduser`.
fn expand_user(path: &str) -> String {
    match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => std::env::var("HOME")
            .map(|home| format!("{home}{rest}"))
            .unwrap_or_else(|_| path.to_owned()),
        _ => path.to_owned(),
    }
}

/// Python's `str(OSError)`: `[Errno N] STRERROR: 'PATH'`.
fn os_error(error: &std::io::Error, path: Option<&str>) -> String {
    let text = error.to_string();
    let Some(code) = error.raw_os_error() else {
        return text;
    };
    let message = text
        .strip_suffix(&format!(" (os error {code})"))
        .unwrap_or(&text);
    match path {
        Some(path) => format!("[Errno {code}] {message}: {}", crate::batch::py_repr(path)),
        None => format!("[Errno {code}] {message}"),
    }
}

/// CPython `netrc._netrclex` over `text`.
struct Lexer<'a> {
    chars: std::str::Chars<'a>,
    line: usize,
    pushback: Vec<String>,
}

impl Lexer<'_> {
    fn read(&mut self) -> Option<char> {
        let c = self.chars.next();
        if c == Some('\n') {
            self.line += 1;
        }
        c
    }

    /// `instream.readline()`: the rest of the line, with its newline.
    fn read_line(&mut self) -> String {
        let mut line = String::new();
        while let Some(c) = self.read() {
            line.push(c);
            if c == '\n' {
                break;
            }
        }
        line
    }

    fn token(&mut self) -> String {
        if !self.pushback.is_empty() {
            return self.pushback.remove(0);
        }
        let whitespace = |c: char| matches!(c, '\n' | '\t' | '\r' | ' ');
        let mut token = String::new();
        while let Some(c) = self.read() {
            if whitespace(c) {
                continue;
            }
            if c == '"' {
                while let Some(c) = self.read() {
                    if c == '"' {
                        return token;
                    }
                    let c = if c == '\\' { self.read() } else { Some(c) };
                    token.extend(c);
                }
            } else {
                let c = if c == '\\' { self.read() } else { Some(c) };
                token.extend(c);
                while let Some(c) = self.read() {
                    if whitespace(c) {
                        return token;
                    }
                    let c = if c == '\\' { self.read() } else { Some(c) };
                    token.extend(c);
                }
            }
        }
        token
    }
}

/// CPython `netrc._parse`: each machine's `(login, password)`;
/// `NetrcParseError` text (`MSG (FILE, line N)`) on failure.
fn parse(text: &str, file: &str) -> std::result::Result<HashMap<String, (String, String)>, String> {
    let mut lexer = Lexer {
        chars: text.chars(),
        line: 1,
        pushback: Vec::new(),
    };
    let error = |message: String, line: usize| format!("{message} ({file}, line {line})");
    let mut hosts = HashMap::new();
    loop {
        let saved_line = lexer.line;
        let toplevel = lexer.token();
        let entry = match toplevel.as_str() {
            "" => break,
            tt if tt.starts_with('#') => {
                if lexer.line == saved_line && tt.len() == 1 {
                    lexer.read_line();
                }
                continue;
            }
            "machine" => lexer.token(),
            "default" => "default".to_owned(),
            "macdef" => {
                lexer.token();
                loop {
                    let line = lexer.read_line();
                    if line.is_empty() {
                        return Err(error(
                            "Macro definition missing null line terminator.".into(),
                            lexer.line,
                        ));
                    }
                    if line == "\n" {
                        break;
                    }
                }
                continue;
            }
            tt => {
                return Err(error(
                    format!("bad toplevel token {}", crate::batch::py_repr(tt)),
                    lexer.line,
                ));
            }
        };
        if entry.is_empty() {
            return Err(error(
                format!("missing {} name", crate::batch::py_repr(&toplevel)),
                lexer.line,
            ));
        }
        let (mut login, mut password) = (String::new(), String::new());
        loop {
            let previous_line = lexer.line;
            let tt = lexer.token();
            if tt.starts_with('#') {
                if lexer.line == previous_line {
                    lexer.read_line();
                }
                continue;
            }
            match tt.as_str() {
                "" | "machine" | "default" | "macdef" => {
                    hosts.insert(entry, (login, password));
                    lexer.pushback.push(tt);
                    break;
                }
                "login" | "user" => login = lexer.token(),
                "account" => {
                    lexer.token();
                }
                "password" => password = lexer.token(),
                _ => {
                    return Err(error(
                        format!("bad follower token {}", crate::batch::py_repr(&tt)),
                        lexer.line,
                    ));
                }
            }
        }
    }
    Ok(hosts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netrc_follows_cpython_parser() {
        let hosts = parse(
            "# comment line\nmachine Comcast_SSO login \"user name\" password p\\\"w\n\ndefault login d password dp\n",
            "f",
        )
        .unwrap();
        assert_eq!(
            hosts["Comcast_SSO"],
            ("user name".to_owned(), "p\"w".to_owned())
        );
        assert_eq!(hosts["default"], ("d".to_owned(), "dp".to_owned()));
        assert_eq!(
            parse("machine a login b\nbogus x\n", "/n").unwrap_err(),
            "bad follower token 'bogus' (/n, line 2)"
        );
        assert_eq!(
            parse("oops\n", "/n").unwrap_err(),
            "bad toplevel token 'oops' (/n, line 2)"
        );
    }
}
