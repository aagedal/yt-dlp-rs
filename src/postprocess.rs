//! Local postprocessing orchestration and no-clobber publication.
use crate::{
    AudioOptions, DownloadOptions, DownloadPhase, DownloadProgress, Error, Result, VideoInfo,
    VideoOptions, YoutubeDl, audio_original_path, audio_output_path,
};
use std::{
    fs,
    ops::ControlFlow,
    path::{Path, PathBuf},
};

/// Local processors run in upstream order: extract audio, then remux/recode.
/// Source and intermediate retention is enabled by either keep flag.
#[derive(Debug, Clone, Default)]
pub struct PostprocessOptions {
    pub audio: Option<AudioOptions>,
    pub video: Option<VideoOptions>,
    pub keep_video: bool,
}

/// Published final media and any sources/intermediates retained by keep-video.
#[derive(Debug, Clone)]
pub struct ProcessedDownload {
    pub path: PathBuf,
    pub bytes: u64,
    /// The original downloaded/merged source, if retained.
    pub kept_video: Option<PathBuf>,
    /// All retained files, including kept_video when present.
    pub kept_files: Vec<PathBuf>,
}

struct Stage {
    disk: PathBuf,
    path: PathBuf,
    ext: String,
    container: String,
    bytes: u64,
}

fn retained_paths(stages: &[Stage], final_path: Option<&Path>) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for (index, stage) in stages.iter().enumerate() {
        let collision = final_path == Some(stage.path.as_path())
            || stages[index + 1..]
                .iter()
                .any(|later| later.path == stage.path);
        let path = if collision {
            audio_original_path(&stage.path)?
        } else {
            stage.path.clone()
        };
        if paths.contains(&path) || final_path == Some(path.as_path()) {
            return Err(Error::Postprocess("retained media paths collide".into()));
        }
        paths.push(path);
    }
    Ok(paths)
}

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
    processors: &PostprocessOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<ProcessedDownload> {
    absent(destination)?;
    let needs_audio = processors
        .audio
        .as_ref()
        .is_some_and(|audio| audio.needs_probe(&info.ext));
    let needs_video = processors
        .video
        .as_ref()
        .map(|video| video.plan(&info.ext))
        .transpose()?
        .is_some_and(|plan| !plan.skip);
    if !needs_audio && !needs_video {
        let bytes = downloader.download_with_progress(info, destination, options, callback)?;
        return Ok(ProcessedDownload {
            path: destination.into(),
            bytes,
            kept_video: None,
            kept_files: Vec::new(),
        });
    }
    let keep = processors.keep_video
        || processors
            .audio
            .as_ref()
            .is_some_and(|audio| audio.keep_video);
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let workspace = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-process-")
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
    let mut stages = vec![Stage {
        disk: input,
        path: destination.into(),
        ext: info.ext.clone(),
        container: info.ext.to_lowercase(),
        bytes,
    }];
    let mut publishing = false;
    let process = (|| -> Result<ProcessedDownload> {
        if needs_audio {
            let current = stages.last().unwrap();
            if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break() {
                return Err(Error::DownloadCancelled);
            }
            let mut observe = || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
            let codec = crate::audio::ffmpeg::probe(
                &probe_path(downloader)?,
                &current.disk,
                &current.container,
                &mut observe,
            )?;
            let plan = processors
                .audio
                .as_ref()
                .unwrap()
                .plan(&current.ext, Some(&codec))?;
            let path = audio_output_path(&current.path, &current.ext, &plan.extension)?;
            if !plan.skip && !(plan.encoder.as_deref() == Some("copy") && path == current.path) {
                absent(&path)?;
                let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                    Error::Postprocess(
                        "audio conversion requires an explicit --ffmpeg-location PATH".into(),
                    )
                })?;
                let output = workspace.path().join(format!("audio.{}", plan.extension));
                let mut observe =
                    || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
                crate::audio::ffmpeg::convert(
                    executable,
                    &current.disk,
                    &current.container,
                    &output,
                    &plan,
                    &mut observe,
                )?;
                let bytes = fs::metadata(&output)?.len();
                // AAC copy deliberately keeps upstream's m4a name with ADTS bytes.
                let container = if plan.muxer == "adts" {
                    "aac".into()
                } else {
                    plan.extension.clone()
                };
                stages.push(Stage {
                    disk: output,
                    path,
                    ext: plan.extension,
                    container,
                    bytes,
                });
            }
        }
        if let Some(video) = &processors.video {
            let current = stages.last().unwrap();
            let plan = video.plan(&current.ext)?;
            if !plan.skip {
                let path =
                    audio_output_path(&current.path, &current.ext.to_lowercase(), &plan.extension)?;
                absent(&path)?;
                if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break() {
                    return Err(Error::DownloadCancelled);
                }
                let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                    Error::Postprocess(
                        "video processing requires an explicit --ffmpeg-location PATH".into(),
                    )
                })?;
                let output = workspace.path().join(format!("video.{}", plan.extension));
                let mut observe =
                    || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
                crate::video::ffmpeg::convert(
                    executable,
                    &current.disk,
                    &current.container,
                    &output,
                    &plan,
                    &mut observe,
                )?;
                let bytes = fs::metadata(&output)?.len();
                stages.push(Stage {
                    disk: output,
                    path,
                    ext: plan.extension.clone(),
                    container: plan.extension,
                    bytes,
                });
            }
        }
        publishing = true;
        let last = stages.last().unwrap();
        let retained = if keep {
            retained_paths(&stages[..stages.len() - 1], Some(&last.path))?
        } else {
            Vec::new()
        };
        absent(&last.path)?;
        for path in &retained {
            absent(path)?;
        }
        if checkpoint(callback, last.bytes, DownloadPhase::Publishing).is_break() {
            return Err(Error::DownloadCancelled);
        }
        for (stage, path) in stages.iter().zip(&retained) {
            publish(&stage.disk, path)?;
        }
        publish(&last.disk, &last.path)?;
        Ok(ProcessedDownload {
            path: last.path.clone(),
            bytes: last.bytes,
            kept_video: retained.first().cloned(),
            kept_files: retained,
        })
    })();
    match process {
        Err(Error::DownloadCancelled) => Err(Error::DownloadCancelled),
        Err(error) => {
            // Retain the latest completed input on failure. Earlier successful
            // processor inputs are retained as well when keep-video is enabled.
            let count = if publishing {
                stages.len().saturating_sub(1).max(1)
            } else {
                stages.len()
            };
            let completed = &stages[..count];
            let paths = retained_paths(completed, None)?;
            let first = if keep { 0 } else { completed.len() - 1 };
            let mut failures = Vec::new();
            for (stage, path) in completed.iter().zip(paths).skip(first) {
                if stage.disk.exists()
                    && let Err(preserve) = publish(&stage.disk, &path)
                {
                    // One retained-path collision must not discard another
                    // completed stage whose destination is still available.
                    failures.push(preserve.to_string());
                }
            }
            if !failures.is_empty() {
                return Err(Error::Postprocess(format!(
                    "{error}; cannot preserve completed media: {}",
                    failures.join("; ")
                )));
            }
            Err(error)
        }
        result => result,
    }
}
