//! Audio download orchestration and no-clobber publication, separate from FFmpeg.
use crate::{
    AudioDownload, AudioOptions, DownloadOptions, DownloadPhase, DownloadProgress, Error, Result,
    VideoInfo, YoutubeDl, audio_original_path, audio_output_path,
};
use std::{
    fs,
    ops::ControlFlow,
    path::{Path, PathBuf},
};

fn absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(Error::OutputExists(path.into())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
fn publish(source: &Path, destination: &Path) -> Result<()> {
    fs::OpenOptions::new()
        .write(true)
        .open(source)?
        .sync_all()?;
    tempfile::TempPath::try_from_path(source)?
        .persist_noclobber(destination)
        .map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::OutputExists(destination.into())
            } else {
                Error::Io(error.error)
            }
        })?;
    Ok(())
}
fn checkpoint(
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
    bytes: u64,
    phase: DownloadPhase,
) -> ControlFlow<()> {
    callback(DownloadProgress {
        attempt: 1,
        downloaded_bytes: bytes,
        total_bytes: Some(bytes),
        phase,
        stream_index: None,
        stream_count: None,
        fragment_index: None,
        fragment_count: None,
    })
}
fn probe_path(downloader: &YoutubeDl) -> Result<PathBuf> {
    if let Some(path) = &downloader.ffprobe {
        return Ok(path.clone());
    }
    let ffmpeg = downloader.ffmpeg.as_ref().ok_or_else(|| Error::Postprocess("audio inspection requires --ffprobe-location PATH or ffprobe beside explicit --ffmpeg-location".into()))?;
    let name = if ffmpeg
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    Ok(ffmpeg.with_file_name(name))
}

pub(crate) fn download(
    downloader: &YoutubeDl,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    audio: &AudioOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<AudioDownload> {
    absent(destination)?;
    if !audio.needs_probe(&info.ext) {
        let bytes = downloader.download_with_progress(info, destination, options, callback)?;
        return Ok(AudioDownload {
            path: destination.into(),
            bytes,
            kept_video: None,
        });
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let workspace = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-audio-")
        .tempdir_in(parent)?;
    let input = workspace.path().join(format!(
        "source.{}",
        crate::output::safe_component(&info.ext)
    ));
    let bytes = downloader.download_with_progress(info, &input, options, |progress| {
        if progress.phase == DownloadPhase::Publishing {
            ControlFlow::Continue(())
        } else {
            callback(progress)
        }
    })?;
    let process = (|| -> Result<AudioDownload> {
        if checkpoint(callback, bytes, DownloadPhase::Postprocessing).is_break() {
            return Err(Error::DownloadCancelled);
        }
        let mut observe = || checkpoint(callback, bytes, DownloadPhase::Postprocessing);
        let codec =
            crate::audio::ffmpeg::probe(&probe_path(downloader)?, &input, &info.ext, &mut observe)?;
        let plan = audio.plan(&info.ext, Some(&codec))?;
        let path = audio_output_path(destination, &info.ext, &plan.extension)?;
        if plan.skip || (plan.encoder.as_deref() == Some("copy") && path == destination) {
            if checkpoint(callback, bytes, DownloadPhase::Publishing).is_break() {
                return Err(Error::DownloadCancelled);
            }
            publish(&input, destination)?;
            return Ok(AudioDownload {
                path: destination.into(),
                bytes,
                kept_video: None,
            });
        }
        absent(&path)?;
        let original = if audio.keep_video {
            Some(if path == destination {
                audio_original_path(destination)?
            } else {
                destination.into()
            })
        } else {
            None
        };
        if let Some(original) = &original {
            absent(original)?;
        }
        let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
            Error::Postprocess(
                "audio conversion requires an explicit --ffmpeg-location PATH".into(),
            )
        })?;
        let output = workspace
            .path()
            .join(format!("converted.{}", plan.extension));
        let mut observe = || checkpoint(callback, bytes, DownloadPhase::Postprocessing);
        crate::audio::ffmpeg::convert(executable, &input, &info.ext, &output, &plan, &mut observe)?;
        let converted_bytes = fs::metadata(&output)?.len();
        if checkpoint(callback, converted_bytes, DownloadPhase::Publishing).is_break() {
            return Err(Error::DownloadCancelled);
        }
        if let Some(original) = &original {
            publish(&input, original)?;
        }
        publish(&output, &path)?;
        Ok(AudioDownload {
            path,
            bytes: converted_bytes,
            kept_video: original,
        })
    })();
    match process {
        Err(Error::DownloadCancelled) => Err(Error::DownloadCancelled),
        Err(error) => {
            // A complete source is useful after failed conversion, as upstream's
            // post-download processing leaves it. Never overwrite a raced output.
            if input.exists()
                && let Err(preserve) = publish(&input, destination)
            {
                return Err(Error::Postprocess(format!(
                    "{error}; cannot preserve source download: {preserve}"
                )));
            }
            Err(error)
        }
        result => result,
    }
}
