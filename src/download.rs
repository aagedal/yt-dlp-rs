use crate::{Error, Result, VideoInfo};
use reqwest::{
    blocking::Client,
    header::{ACCEPT_ENCODING, REFERER},
};
use std::{fs, io::Write, path::Path};

/// Stream to a same-directory temporary file, then publish without overwriting.
/// Failed and truncated transfers leave no final file or abandoned temporary file.
pub fn download(client: &Client, info: &VideoInfo, destination: &Path) -> Result<u64> {
    if destination.exists() {
        return Err(Error::OutputExists(destination.into()));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut request = client
        .get(info.url.clone())
        .header(ACCEPT_ENCODING, "identity");
    if !info.direct {
        request = request.header(REFERER, info.webpage_url.as_str());
    }
    let mut response = request.send()?.error_for_status()?;
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(Error::PartialResponse);
    }
    if !info.direct && crate::extractor::direct_media_info(&response, &info.url).is_none() {
        return Err(Error::UnsupportedHtml5(
            "source no longer serves supported direct HTTP media".into(),
        ));
    }
    let expected = response.content_length();
    let mut temporary = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-")
        .suffix(".part")
        .tempfile_in(parent)?;
    let actual = std::io::copy(&mut response, &mut temporary)?;
    if let Some(expected) = expected
        && actual != expected
    {
        return Err(Error::IncompleteDownload { expected, actual });
    }
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(destination).map_err(|e| {
        if e.error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::OutputExists(destination.into())
        } else {
            Error::Io(e.error)
        }
    })?;
    Ok(actual)
}
