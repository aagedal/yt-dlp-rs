use crate::{Error, Result};
use reqwest::{
    blocking::Response,
    header::{HeaderMap, RETRY_AFTER},
};
use std::time::Duration;

const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

// Support the unambiguous delay-seconds form only. Duplicate fields and dates
// fall back to local backoff; never combine multiple server instructions.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let mut values = headers.get_all(RETRY_AFTER).iter();
    let value = values.next()?.to_str().ok()?.trim_matches([' ', '\t']);
    if values.next().is_some() || value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Saturate while parsing so even an overflowing decimal remains bounded.
    let seconds = value.bytes().fold(0_u64, |seconds, digit| {
        seconds
            .saturating_mul(10)
            .saturating_add(u64::from(digit - b'0'))
    });
    Some(Duration::from_secs(seconds).min(MAX_RETRY_AFTER))
}

pub(crate) fn check_response(response: Response) -> Result<Response> {
    if let Err(source) = response.error_for_status_ref() {
        if transient_request(&source)
            && let Some(retry_after) = retry_after(response.headers())
        {
            return Err(Error::HttpRetryAfter {
                source,
                retry_after,
            });
        }
        return Err(Error::Http(source));
    }
    Ok(response)
}

pub(crate) fn failure_delay(error: &Error) -> Option<Duration> {
    match error {
        Error::Http(error) if transient_request(error) => Some(Duration::ZERO),
        Error::HttpRetryAfter {
            source,
            retry_after,
        } if transient_request(source) => Some(*retry_after),
        Error::ExtractionRead(_) => Some(Duration::ZERO),
        _ => None,
    }
}

fn delay(attempt: u8, retry_after: Duration) -> Duration {
    let delay_ms = (100_u64 << attempt.min(4)).min(1000);
    Duration::from_millis(delay_ms).max(retry_after.min(MAX_RETRY_AFTER))
}

pub(crate) fn transient_request(error: &reqwest::Error) -> bool {
    error.is_connect()
        || error.is_timeout()
        || error
            .status()
            .is_some_and(|status| matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504))
}

pub(crate) fn backoff(attempt: u8, retry_after: Duration) {
    // Bounded exponential backoff: 100, 200, 400, 800, then 1000 ms.
    std::thread::sleep(delay(attempt, retry_after));
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn delay_seconds_are_strict_and_bounded() {
        for (value, seconds) in [
            ("0", 0),
            ("0002", 2),
            (" \t12 \t", 12),
            ("60", 60),
            ("61", 60),
            ("184467440737095516160000", 60),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, HeaderValue::from_str(value).unwrap());
            assert_eq!(
                retry_after(&headers),
                Some(Duration::from_secs(seconds)),
                "{value:?}"
            );
        }
        for value in [
            "",
            " \t",
            "+1",
            "-1",
            "1.0",
            "1, 2",
            "1 2",
            "Wed, 21 Oct 2015 07:28:00 GMT",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, HeaderValue::from_str(value).unwrap());
            assert_eq!(retry_after(&headers), None, "{value:?}");
        }
        let mut headers = HeaderMap::new();
        assert_eq!(retry_after(&headers), None);
        headers.append(RETRY_AFTER, HeaderValue::from_static("1"));
        headers.append(RETRY_AFTER, HeaderValue::from_static("1"));
        assert_eq!(retry_after(&headers), None);
        headers.clear();
        headers.insert(RETRY_AFTER, HeaderValue::from_bytes(&[0xff]).unwrap());
        assert_eq!(retry_after(&headers), None);
    }

    #[test]
    fn server_hint_extends_backoff_with_a_fixed_cap() {
        for (attempt, milliseconds) in [
            (0, 100),
            (1, 200),
            (2, 400),
            (3, 800),
            (4, 1000),
            (255, 1000),
        ] {
            assert_eq!(
                delay(attempt, Duration::ZERO),
                Duration::from_millis(milliseconds)
            );
            assert_eq!(
                delay(attempt, Duration::from_secs(2)),
                Duration::from_secs(2)
            );
            assert_eq!(delay(attempt, Duration::MAX), MAX_RETRY_AFTER);
        }
    }
}
