//! Python's `netrc` module (3.11+) as upstream uses it: `netrc.netrc(FILE)`
//! and `netrc_from_content(TEXT)` (file `-`), with `machine`/`default`
//! entries (`login`/`user`, `account`, `password`), `macdef` macros,
//! quoted and backslash-escaped tokens, `#` comments and `NetrcParseError`'s
//! `MSG (FILE, line N)` text. Like upstream (which always names the file),
//! the `~/.netrc` ownership and permission check does not apply.
use std::collections::{HashMap, VecDeque};

/// `(login, account, password)`.
pub type Authenticator = (String, String, String);

/// Upstream `NetrcParseError`: `MSG (FILE, line N)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub msg: String,
    pub file: String,
    pub lineno: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}, line {})", self.msg, self.file, self.lineno)
    }
}

impl std::error::Error for ParseError {}

/// A parsed netrc: `hosts` (later entries replace earlier ones) and
/// `macros`.
#[derive(Debug, Default)]
pub struct Netrc {
    pub hosts: HashMap<String, Authenticator>,
    pub macros: HashMap<Option<String>, Vec<String>>,
}

/// `_netrclex`: tokens read one character at a time (counting lines), with
/// `readline` on the same stream (which, like Python's, counts no line).
struct Lexer {
    chars: Vec<char>,
    pos: usize,
    lineno: usize,
    pushback: VecDeque<Option<String>>,
}

const WHITESPACE: &[char] = &['\n', '\t', '\r', ' '];

impl Lexer {
    fn read_char(&mut self) -> Option<char> {
        let ch = *self.chars.get(self.pos)?;
        self.pos += 1;
        if ch == '\n' {
            self.lineno += 1;
        }
        Some(ch)
    }

    fn readline(&mut self) -> String {
        let mut line = String::new();
        while let Some(&ch) = self.chars.get(self.pos) {
            self.pos += 1;
            line.push(ch);
            if ch == '\n' {
                break;
            }
        }
        line
    }

    fn get_token(&mut self) -> Option<String> {
        if let Some(token) = self.pushback.pop_front() {
            return token;
        }
        while let Some(ch) = self.read_char() {
            if WHITESPACE.contains(&ch) {
                continue;
            }
            let mut token = String::new();
            if ch == '"' {
                while let Some(ch) = self.read_char() {
                    match ch {
                        '"' => return Some(token),
                        '\\' => token.extend(self.read_char()),
                        ch => token.push(ch),
                    }
                }
            } else {
                match ch {
                    '\\' => token.extend(self.read_char()),
                    ch => token.push(ch),
                }
                while let Some(ch) = self.read_char() {
                    match ch {
                        ch if WHITESPACE.contains(&ch) => return Some(token),
                        '\\' => token.extend(self.read_char()),
                        ch => token.push(ch),
                    }
                }
            }
            return Some(token);
        }
        None
    }
}

impl Netrc {
    /// `netrc.netrc(path)`: an unreadable file is the `OSError` text
    /// (`[Errno 2] No such file or directory: 'PATH'`). Invalid UTF-8 is
    /// replaced (upstream retries with the locale encoding).
    pub fn from_file(path: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| os_error_text(&error, path))?;
        Self::parse(&String::from_utf8_lossy(&bytes), path).map_err(|error| error.to_string())
    }

    /// `netrc_from_content(text)`.
    pub fn from_content(text: &str) -> Result<Self, ParseError> {
        Self::parse(text, "-")
    }

    fn parse(text: &str, file: &str) -> Result<Self, ParseError> {
        let mut netrc = Netrc::default();
        let mut lexer = Lexer {
            chars: text.chars().collect(),
            pos: 0,
            lineno: 1,
            pushback: VecDeque::new(),
        };
        let error = |msg: String, lineno: usize| ParseError {
            msg,
            file: file.to_owned(),
            lineno,
        };
        loop {
            let saved_lineno = lexer.lineno;
            let Some(tt) = lexer.get_token() else {
                break;
            };
            let entryname = if tt.starts_with('#') {
                if lexer.lineno == saved_lineno && tt.chars().count() == 1 {
                    lexer.readline();
                }
                continue;
            } else if tt == "machine" {
                lexer.get_token()
            } else if tt == "default" {
                Some("default".to_owned())
            } else if tt == "macdef" {
                let name = lexer.get_token();
                let mut lines = Vec::new();
                loop {
                    let line = lexer.readline();
                    if line.is_empty() {
                        return Err(error(
                            "Macro definition missing null line terminator.".into(),
                            lexer.lineno,
                        ));
                    }
                    if line == "\n" {
                        break;
                    }
                    lines.push(line);
                }
                netrc.macros.insert(name, lines);
                continue;
            } else {
                return Err(error(
                    format!("bad toplevel token {}", crate::batch::py_repr(&tt)),
                    lexer.lineno,
                ));
            };
            let Some(entryname) = entryname.filter(|name| !name.is_empty()) else {
                return Err(error(
                    format!("missing {} name", crate::batch::py_repr(&tt)),
                    lexer.lineno,
                ));
            };
            let (mut login, mut account, mut password) =
                (String::new(), String::new(), String::new());
            loop {
                let prev_lineno = lexer.lineno;
                let tt = lexer.get_token();
                if let Some(comment) = tt.as_deref().filter(|tt| tt.starts_with('#')) {
                    let _ = comment;
                    if lexer.lineno == prev_lineno {
                        lexer.readline();
                    }
                    continue;
                }
                match tt.as_deref() {
                    None | Some("machine" | "default" | "macdef") => {
                        netrc.hosts.insert(entryname, (login, account, password));
                        lexer.pushback.push_back(tt);
                        break;
                    }
                    Some("login" | "user") => login = lexer.get_token().unwrap_or_default(),
                    Some("account") => account = lexer.get_token().unwrap_or_default(),
                    Some("password") => password = lexer.get_token().unwrap_or_default(),
                    Some(other) => {
                        return Err(error(
                            format!("bad follower token {}", crate::batch::py_repr(other)),
                            lexer.lineno,
                        ));
                    }
                }
            }
        }
        Ok(netrc)
    }

    /// `authenticators(host)`: the host's entry, else `default`'s.
    pub fn authenticators(&self, host: &str) -> Option<&Authenticator> {
        self.hosts.get(host).or_else(|| self.hosts.get("default"))
    }
}

/// Python's `OSError` text for a failed open: `[Errno N] STRERROR: 'PATH'`.
pub(crate) fn os_error_text(error: &std::io::Error, path: &str) -> String {
    match error.raw_os_error() {
        Some(code) => {
            let text = error.to_string();
            let strerror = text
                .strip_suffix(&format!(" (os error {code})"))
                .unwrap_or(&text);
            format!("[Errno {code}] {strerror}: {}", crate::batch::py_repr(path))
        }
        None => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(login: &str, account: &str, password: &str) -> Authenticator {
        (login.into(), account.into(), password.into())
    }

    #[test]
    fn entries_follow_pythons_netrc() {
        // Cases from CPython's test_netrc.
        let netrc = Netrc::from_content(
            "machine host1.domain.com login log1 password pass1 account acct1\n\
             default login log2 password pass2\n\
             macdef macro1\nline1\nline2\n\n\
             machine host.domain.com\tuser\tlog3 password \"pa ss\\\"3\"\n",
        )
        .unwrap();
        assert_eq!(
            netrc.authenticators("host1.domain.com"),
            Some(&entry("log1", "acct1", "pass1"))
        );
        assert_eq!(
            netrc.authenticators("foo.domain.com"),
            Some(&entry("log2", "", "pass2"))
        );
        assert_eq!(
            netrc.authenticators("host.domain.com"),
            Some(&entry("log3", "", "pa ss\"3"))
        );
        assert_eq!(
            netrc.macros[&Some("macro1".into())],
            vec!["line1\n".to_owned(), "line2\n".to_owned()]
        );
        // Escapes outside quotes, and a later entry replacing an earlier one.
        let netrc =
            Netrc::from_content("machine m login a\\ b password x\nmachine m login c").unwrap();
        assert_eq!(netrc.authenticators("m"), Some(&entry("c", "", "")));
        assert_eq!(Netrc::from_content("").unwrap().authenticators("m"), None);
    }

    #[test]
    fn comments_skip_like_pythons_lexer() {
        let netrc = Netrc::from_content(
            "# comment\nmachine foo.domain.com login bar password pass # comment\n\
             #machine skipped\nmachine bar.domain.com login foo password pass\n",
        )
        .unwrap();
        assert_eq!(
            netrc.authenticators("foo.domain.com"),
            Some(&entry("bar", "", "pass"))
        );
        assert_eq!(
            netrc.authenticators("bar.domain.com"),
            Some(&entry("foo", "", "pass"))
        );
        // A token starting with `#` mid-entry ends the line's parsing.
        let netrc = Netrc::from_content("machine m login u #password p\n").unwrap();
        assert_eq!(netrc.authenticators("m"), Some(&entry("u", "", "")));
    }

    #[test]
    fn errors_carry_pythons_text_and_line() {
        let error = |text: &str| Netrc::from_content(text).unwrap_err().to_string();
        assert_eq!(error("foo bar"), "bad toplevel token 'foo' (-, line 1)");
        assert_eq!(
            error("machine m\n  login u\n  port 1\n"),
            "bad follower token 'port' (-, line 3)"
        );
        assert_eq!(error("machine"), "missing 'machine' name (-, line 1)");
        assert_eq!(
            error("macdef m\nline\n"),
            "Macro definition missing null line terminator. (-, line 2)"
        );
    }

    #[test]
    fn unreadable_files_are_os_errors() {
        let error = Netrc::from_file("/nonexistent-netrc-dir/.netrc").unwrap_err();
        assert_eq!(
            error,
            "[Errno 2] No such file or directory: '/nonexistent-netrc-dir/.netrc'"
        );
    }
}
