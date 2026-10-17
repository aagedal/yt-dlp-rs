use std::time::Duration;

pub(crate) fn transient_request(error: &reqwest::Error) -> bool {
    error.is_connect()
        || error.is_timeout()
        || error
            .status()
            .is_some_and(|status| matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504))
}

pub(crate) fn backoff(attempt: u8) {
    // Bounded exponential backoff: 100, 200, 400, 800, then 1000 ms.
    let delay_ms = (100_u64 << attempt.min(4)).min(1000);
    std::thread::sleep(Duration::from_millis(delay_ms));
}
