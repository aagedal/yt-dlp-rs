//! `--ignore-no-formats-error` (`ignore_no_formats_error`): upstream's
//! `InfoExtractor.raise_no_formats` and `YoutubeDL.raise_no_formats`.

use std::sync::atomic::{AtomicBool, Ordering};

static IGNORE: AtomicBool = AtomicBool::new(false);
static WAIT_FOR_VIDEO: AtomicBool = AtomicBool::new(false);

/// `--ignore-no-formats-error`, and whether `--wait-for-video` is given
/// (extractors' expected no-formats errors then warn too).
pub fn set(ignore: bool, wait_for_video: bool) {
    IGNORE.store(ignore, Ordering::Relaxed);
    WAIT_FOR_VIDEO.store(wait_for_video, Ordering::Relaxed);
}

/// `params.get('ignore_no_formats_error')`.
pub fn ignore() -> bool {
    IGNORE.load(Ordering::Relaxed)
}

/// `get_param('ignore_no_formats_error') or get_param('wait_for_video')`.
pub(crate) fn ignored() -> bool {
    ignore() || WAIT_FOR_VIDEO.load(Ordering::Relaxed)
}

/// `InfoExtractor.raise_no_formats(msg, expected=True, video_id=ID)`: a
/// `[EXTRACTOR] ID: MSG` warning (`[EXTRACTOR] MSG` without a `video_id`)
/// when ignored, else the expected error, whose ID defaults to `extract`'s
/// `temp_id`.
pub(crate) fn raise_expected(
    extractor: &str,
    video_id: Option<&str>,
    temp_id: Option<&str>,
    message: &str,
) -> crate::Result<()> {
    let text = |id: Option<&str>| match id {
        Some(id) => format!("[{extractor}] {id}: {message}"),
        None => format!("[{extractor}] {message}"),
    };
    if ignored() {
        crate::screen::warning(&text(video_id));
        return Ok(());
    }
    Err(crate::Error::Reported(text(video_id.or(temp_id))))
}
