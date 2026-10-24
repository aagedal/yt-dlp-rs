//! `_get_login_info('ap_username', 'ap_password', netrc_machine)`: the
//! `--ap-*` options, else the TV provider's netrc machine through the
//! shared login subsystem (`-n`, `--netrc-location`, `--netrc-cmd`).
use crate::Result;

/// The account options `_get_login_info` reads.
#[derive(Clone, Debug, Default)]
pub struct LoginOptions {
    pub username: Option<String>,
    pub password: Option<String>,
}

/// `_get_login_info`: `(username, password)` from the options, else netrc
/// (`[EXTRACTOR] ` prefixes its messages).
pub(super) fn login_info(
    options: &LoginOptions,
    machine: &str,
    extractor: &str,
) -> Result<(Option<String>, Option<String>)> {
    if options.username.is_some() {
        return Ok((options.username.clone(), options.password.clone()));
    }
    Ok(
        match crate::extractor::login::netrc_login_info(extractor, Some(machine))? {
            Some((login, password)) => (Some(login), Some(password)),
            None => (None, None),
        },
    )
}
