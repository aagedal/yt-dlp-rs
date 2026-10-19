//! Local postprocessing orchestration and no-clobber publication.
use crate::{
    AudioOptions, DownloadOptions, DownloadPhase, DownloadProgress, Error, Result,
    SubtitleEmbedding, VideoInfo, VideoOptions, YoutubeDl, audio_original_path, audio_output_path,
};
use std::{
    fs,
    io::Read,
    ops::ControlFlow,
    path::{Path, PathBuf},
};

/// Local processors run in upstream order: audio, remux/recode, then subtitles.
/// Source and intermediate retention is enabled by either keep flag.
#[derive(Debug, Clone, Default)]
pub struct PostprocessOptions {
    pub audio: Option<AudioOptions>,
    pub video: Option<VideoOptions>,
    pub keep_video: bool,
    pub embed: Option<SubtitleEmbedding>,
    /// `--embed-metadata`: upstream `FFmpegMetadataPP` after subtitle embedding.
    pub metadata: bool,
    /// `--embed-chapters` (implied by `--embed-metadata`): the same processor
    /// adds the info dict's chapters.
    pub chapters: bool,
    /// `--embed-info-json` (`if_exists` with `--embed-metadata`): attach the
    /// info JSON to mkv/mka files.
    pub infojson: Option<InfoJsonEmbed>,
    /// Upstream `fixup()` before the processors; `None` skips fixups.
    pub fixup: Option<crate::FixupOptions>,
    /// `--embed-thumbnail`: a local thumbnail embedded after metadata.
    pub thumbnail: Option<PathBuf>,
    /// `--ignore-errors`: a failed processor is reported and skipped, and the
    /// remaining processors continue with the latest completed file.
    pub ignore_errors: bool,
    /// An already-downloaded file (the destination) to process instead of
    /// downloading, like upstream's `post_process` after `has already been
    /// downloaded`. It is replaced in place by same-named results and
    /// removed after a successful conversion unless kept.
    pub existing: Option<PathBuf>,
}

/// Upstream `FFmpegMetadataPP` `add_infojson`.
#[derive(Debug, Clone, Default)]
pub struct InfoJsonEmbed {
    /// The info JSON written for this video, if any.
    pub written: Option<PathBuf>,
    /// `True` (`--embed-info-json`) rather than `if_exists`: a missing info
    /// JSON is written from `json`, and other containers report that it
    /// can only be attached to mkv/mka files.
    pub explicit: bool,
    pub json: Vec<u8>,
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
    pub warnings: Vec<String>,
    /// Processor failures skipped under `ignore_errors`.
    pub errors: Vec<String>,
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
    let existing = processors.existing.as_deref();
    if existing.is_none() {
        absent(destination)?;
    }
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
    if !needs_audio
        && !needs_video
        && processors.embed.is_none()
        && !processors.metadata
        && !processors.chapters
        && processors.infojson.is_none()
        && processors.thumbnail.is_none()
    {
        let bytes = match existing {
            Some(existing) => fs::metadata(existing)?.len(),
            None => downloader.download_with_progress(info, destination, options, callback)?,
        };
        // Upstream still runs the processors, which report why they skip.
        if let Some(audio) = &processors.audio {
            let reason = match audio.target(&info.ext) {
                None => format!("could not find a mapping for {}", info.ext),
                Some(_) => "the file is already in a common audio format".into(),
            };
            crate::screen::screen(&format!(
                "[ExtractAudio] Not converting audio {}; {reason}",
                destination.display()
            ));
        }
        if let Some(video) = &processors.video
            && let Some(reason) = video.skip_reason(&info.ext)
        {
            let (action, name) = video.action();
            crate::screen::screen(&format!(
                "[{name}] Not {action} media file \"{}\"; {reason}",
                destination.display()
            ));
        }
        return Ok(ProcessedDownload {
            path: destination.into(),
            bytes,
            kept_video: None,
            kept_files: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
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
    // Upstream merges selected formats from `NAME.fID.EXT` files beside the
    // destination (printing their downloads and the merge) before the
    // processors run; the merged file then becomes the private source.
    let upstream_merge = existing.is_none()
        && !info.requested_formats.is_empty()
        && matches!(options.merge_files, crate::MergeFiles::Upstream { .. })
        && !downloader.uses_ffmpeg_fd(info);
    let bytes = if let Some(existing) = existing {
        // A private link (or copy) of the existing file is the source.
        if fs::hard_link(existing, &input).is_err() {
            fs::copy(existing, &input)?;
        }
        fs::metadata(&input)?.len()
    } else if upstream_merge {
        // Each stream's download reports its own completion.
        let bytes =
            downloader.download_with_progress(info, destination, options, &mut *callback)?;
        fs::rename(destination, &input)?;
        bytes
    } else {
        // The workspace source is private; part files apply to unprocessed
        // media only.
        let options = DownloadOptions {
            part_files: crate::PartFiles::Private,
            merge_files: crate::MergeFiles::Private,
            ..options
        };
        downloader.download_with_progress(info, &input, options, |progress| {
            if progress.phase == DownloadPhase::Publishing {
                ControlFlow::Continue(())
            } else {
                callback(progress)
            }
        })?
    };
    // Upstream `process_info` fixes the download (after its progress ends)
    // before `post_process` and its prints.
    // Like upstream's default policy, existing files are not fixed again.
    if let Some(fixup) = &processors.fixup
        && !upstream_merge
        && existing.is_none()
    {
        if checkpoint(callback, bytes, DownloadPhase::Completed).is_break() {
            return Err(Error::DownloadCancelled);
        }
        downloader.fixup(info, &input, destination, fixup)?;
    }
    // End the transfer's progress before processors print (upstream reports
    // the download finished before running postprocessors).
    if checkpoint(callback, bytes, DownloadPhase::Postprocessing).is_break() {
        return Err(Error::DownloadCancelled);
    }
    // HLS media is labeled `mp4` like upstream; FFmpeg reads its real container.
    let container = crate::container::sniff(&input)
        .map(str::to_owned)
        .unwrap_or_else(|| info.ext.to_lowercase());
    let source = input.clone();
    // Publish a stage's file to `path`: an unchanged existing source stays in
    // place, and results named like the existing file replace it.
    let place = |disk: &Path, path: &Path| -> Result<()> {
        if existing == Some(path) {
            if disk != source {
                fs::rename(disk, path)?;
            }
            return Ok(());
        }
        if disk == source
            && let Some(existing) = existing
        {
            absent(path)?;
            return Ok(fs::rename(existing, path)?);
        }
        publish(disk, path)
    };
    let mut stages = vec![Stage {
        disk: input,
        path: destination.into(),
        ext: info.ext.clone(),
        container,
        bytes,
    }];
    let mut publishing = false;
    let mut delete_subtitles = Vec::new();
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    // Upstream `run_pp` deletion notice for a processor's replaced input.
    let deleted = |path: &Path| {
        if !keep {
            crate::screen::screen(&format!(
                "Deleting original file {} (pass -k to keep)",
                path.display()
            ));
        }
    };
    let process = (|| -> Result<ProcessedDownload> {
        // Upstream `run_pp`: with --ignore-errors a failed processor is
        // reported and skipped without its file deletions.
        macro_rules! stage {
            ($body:block) => {{
                let subtitle_count = delete_subtitles.len();
                let outcome = (|| -> Result<()> {
                    $body
                    Ok(())
                })();
                match outcome {
                    Ok(()) => {}
                    Err(Error::DownloadCancelled) => return Err(Error::DownloadCancelled),
                    Err(error) if processors.ignore_errors => {
                        delete_subtitles.truncate(subtitle_count);
                        errors.push(error.to_string());
                    }
                    Err(error) => return Err(error),
                }
            }};
        }
        stage!({
            if needs_audio {
                let current = stages.last().unwrap();
                if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break() {
                    return Err(Error::DownloadCancelled);
                }
                let mut observe =
                    || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
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
                let options = processors.audio.as_ref().unwrap();
                if plan.skip {
                    // Upstream `Not converting audio PATH; REASON`.
                    let reason = match options.target(&current.ext) {
                        None => format!("could not find a mapping for {}", current.ext),
                        Some(_) => "the file is already in a common audio format".into(),
                    };
                    crate::screen::screen(&format!(
                        "[ExtractAudio] Not converting audio {}; {reason}",
                        current.path.display()
                    ));
                } else if plan.encoder.as_deref() == Some("copy") && path == current.path {
                    crate::screen::screen(&format!(
                        "[ExtractAudio] Not converting audio {}; file is already in target format {}",
                        current.path.display(),
                        options.target(&current.ext).unwrap_or("best")
                    ));
                }
                if !plan.skip && !(plan.encoder.as_deref() == Some("copy") && path == current.path)
                {
                    crate::screen::screen(&format!(
                        "[ExtractAudio] Destination: {}",
                        path.display()
                    ));
                    if existing != Some(path.as_path()) {
                        absent(&path)?;
                    }
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
                    // Upstream FFmpeg outputs take the oldest input mtime.
                    crate::filetime::inherit(
                        &output,
                        crate::filetime::oldest([current.disk.as_path()]),
                    );
                    let bytes = fs::metadata(&output)?.len();
                    // AAC copy deliberately keeps upstream's m4a name with ADTS bytes.
                    let container = if plan.muxer == "adts" {
                        "aac".into()
                    } else {
                        plan.extension.clone()
                    };
                    // A same-named output moves the input to `NAME.orig.EXT`.
                    let original = if path == current.path {
                        audio_original_path(&current.path)?
                    } else {
                        current.path.clone()
                    };
                    stages.push(Stage {
                        disk: output,
                        path,
                        ext: plan.extension,
                        container,
                        bytes,
                    });
                    deleted(&original);
                }
            }
        });
        stage!({
            if let Some(video) = &processors.video {
                let current = stages.last().unwrap();
                let plan = video.plan(&current.ext)?;
                let (action, name) = video.action();
                if let Some(reason) = video.skip_reason(&current.ext).filter(|_| plan.skip) {
                    crate::screen::screen(&format!(
                        "[{name}] Not {action} media file \"{}\"; {reason}",
                        current.path.display()
                    ));
                }
                if !plan.skip {
                    let path = audio_output_path(
                        &current.path,
                        &current.ext.to_lowercase(),
                        &plan.extension,
                    )?;
                    let mut title = action.to_owned();
                    title[..1].make_ascii_uppercase();
                    crate::screen::screen(&format!(
                        "[{name}] {title} video from {} to {}; Destination: {}",
                        current.ext.to_lowercase(),
                        plan.extension,
                        path.display()
                    ));
                    absent(&path)?;
                    if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break()
                    {
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
                    // Upstream FFmpeg outputs take the oldest input mtime.
                    crate::filetime::inherit(
                        &output,
                        crate::filetime::oldest([current.disk.as_path()]),
                    );
                    let bytes = fs::metadata(&output)?.len();
                    let original = current.path.clone();
                    stages.push(Stage {
                        disk: output,
                        path,
                        ext: plan.extension.clone(),
                        container: plan.extension,
                        bytes,
                    });
                    deleted(&original);
                }
            }
        });
        stage!({
            if let Some(embedding) = &processors.embed {
                let subtitle_start = delete_subtitles.len();
                let current = stages.last_mut().unwrap();
                let mut plan = crate::embed_subtitles::plan(&current.ext, &embedding.subtitles)?;
                warnings.append(&mut plan.warnings);
                warnings.append(&mut plan.messages);
                if !plan.skip {
                    if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break()
                    {
                        return Err(Error::DownloadCancelled);
                    }
                    let mut total = 0u64;
                    let mut seen = std::collections::HashSet::new();
                    // Upstream FFmpeg inputs are the original subtitle files.
                    let mut oldest = crate::filetime::oldest([current.disk.as_path()]);
                    for (index, file) in plan.files.iter_mut().enumerate() {
                        if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing)
                            .is_break()
                        {
                            return Err(Error::DownloadCancelled);
                        }
                        let metadata = subtitle_metadata(&file.path)?;
                        oldest = oldest.into_iter().chain(metadata.modified().ok()).min();
                        if metadata.len() > 16 * 1024 * 1024 {
                            return Err(Error::Postprocess(
                                "subtitle embedding input exceeds 16 MiB".into(),
                            ));
                        }
                        let canonical = file.path.canonicalize()?;
                        let mut bytes = Vec::new();
                        fs::File::open(&file.path)?
                            .take(16 * 1024 * 1024 + 1)
                            .read_to_end(&mut bytes)?;
                        total += bytes.len() as u64;
                        if bytes.len() > 16 * 1024 * 1024 || total > 256 * 1024 * 1024 {
                            return Err(Error::Postprocess(
                                "subtitle embedding inputs exceed byte budget".into(),
                            ));
                        }
                        if !same_subtitle(&metadata, &subtitle_metadata(&file.path)?) {
                            return Err(Error::Postprocess(
                                "subtitle changed during embedding".into(),
                            ));
                        }
                        if !embedding.keep_subtitles && !keep && seen.insert(canonical) {
                            delete_subtitles.push((file.path.clone(), metadata));
                        }
                        let private = workspace
                            .path()
                            .join(format!("subtitle-{index}.{}", file.ext));
                        fs::write(&private, bytes)?;
                        file.path = private;
                    }
                    let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                        Error::Postprocess(
                            "subtitle embedding requires an explicit --ffmpeg-location PATH".into(),
                        )
                    })?;
                    crate::screen::screen(&format!(
                        "[EmbedSubtitle] Embedding subtitles in \"{}\"",
                        current.path.display()
                    ));
                    let output = workspace.path().join(format!("embedded.{}", current.ext));
                    let mut observe =
                        || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
                    crate::embed_subtitles::ffmpeg::embed(
                        executable,
                        &current.disk,
                        &current.container,
                        &output,
                        &plan,
                        &mut observe,
                    )?;
                    crate::filetime::inherit(&output, oldest);
                    // Upstream replaces the current media and returns no media deletion.
                    // Keep-video therefore retains earlier processors' inputs, not a
                    // new unembedded copy of this stage.
                    current.bytes = fs::metadata(&output)?.len();
                    current.disk = output;
                    current.container = current.ext.clone();
                    // Upstream returns the embedded subtitle files for deletion.
                    for (path, _) in &delete_subtitles[subtitle_start..] {
                        crate::screen::screen(&format!(
                            "Deleting original file {} (pass -k to keep)",
                            path.display()
                        ));
                    }
                }
            }
        });
        stage!({
            if processors.metadata || processors.chapters || processors.infojson.is_some() {
                let current = stages.last_mut().unwrap();
                let chapters = processors.chapters && !info.metadata.chapters.is_empty();
                // `_get_infojson_opts`: only mkv/mka files take the attachment.
                let mut attachment = None;
                if let Some(embed) = &processors.infojson {
                    if matches!(current.ext.to_lowercase().as_str(), "mkv" | "mka") {
                        attachment = match embed.written.as_ref().filter(|path| path.is_file()) {
                            // FFmpeg runs in the workspace, so name it absolutely.
                            Some(path) => Some(path.canonicalize()?),
                            None if embed.explicit => {
                                let path = workspace.path().join("info.json.temp");
                                fs::write(&path, &embed.json)?;
                                Some(path)
                            }
                            None => None,
                        };
                    } else if embed.explicit {
                        crate::screen::screen(
                            "[Metadata] The info-json can only be attached to mkv/mka files",
                        );
                    }
                }
                let Some(args) = crate::embed_metadata::plan_args(
                    info,
                    &current.ext,
                    processors.metadata,
                    chapters,
                    attachment.as_deref(),
                ) else {
                    crate::screen::screen("[Metadata] There isn't any metadata to add");
                    return Ok(());
                };
                if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break() {
                    return Err(Error::DownloadCancelled);
                }
                let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                    Error::Postprocess(
                        "metadata embedding requires an explicit --ffmpeg-location PATH".into(),
                    )
                })?;
                // Upstream writes `NAME.meta` beside the media and deletes it.
                let metadata_input = if chapters {
                    let path = workspace.path().join("chapters.meta");
                    fs::write(
                        &path,
                        crate::embed_metadata::ffmetadata(&info.metadata.chapters),
                    )?;
                    Some(path)
                } else {
                    None
                };
                let plan = crate::video::VideoPlan {
                    extension: current.ext.clone(),
                    args,
                    muxer: crate::embed_metadata::muxer(&current.ext.to_lowercase()).into(),
                    skip: false,
                    metadata_input,
                    pp: "Metadata",
                };
                crate::screen::screen(&format!(
                    "[Metadata] Adding metadata to \"{}\"",
                    current.path.display()
                ));
                let output = workspace.path().join(format!("metadata.{}", current.ext));
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
                crate::filetime::inherit(
                    &output,
                    crate::filetime::oldest([current.disk.as_path()]),
                );
                // Like upstream, metadata replaces the current file in place.
                current.bytes = fs::metadata(&output)?.len();
                current.disk = output;
                current.container = current.ext.to_lowercase();
            }
        });
        stage!({
            if let Some(thumbnail) = &processors.thumbnail {
                let current = stages.last_mut().unwrap();
                if checkpoint(callback, current.bytes, DownloadPhase::Postprocessing).is_break() {
                    return Err(Error::DownloadCancelled);
                }
                let executable = downloader.ffmpeg.as_ref().ok_or_else(|| {
                    Error::Postprocess(
                        "thumbnail embedding requires an explicit --ffmpeg-location PATH".into(),
                    )
                })?;
                let output = workspace
                    .path()
                    .join(format!("thumbnail-embedded.{}", current.ext));
                let mut observe =
                    || checkpoint(callback, current.bytes, DownloadPhase::Postprocessing);
                crate::thumbnail_process::embed(
                    executable,
                    &probe_path(downloader)?,
                    &current.disk,
                    &current.ext.to_lowercase(),
                    thumbnail,
                    workspace.path(),
                    &output,
                    &mut observe,
                )?;
                // Upstream restores the media's own modification time.
                crate::filetime::inherit(
                    &output,
                    crate::filetime::oldest([current.disk.as_path()]),
                );
                current.bytes = fs::metadata(&output)?.len();
                current.disk = output;
                current.container = current.ext.to_lowercase();
            }
        });
        publishing = true;
        let last = stages.last().unwrap();
        let retained = if keep {
            retained_paths(&stages[..stages.len() - 1], Some(&last.path))?
        } else {
            Vec::new()
        };
        if existing != Some(last.path.as_path()) {
            absent(&last.path)?;
        }
        for path in &retained {
            if existing != Some(path.as_path()) {
                absent(path)?;
            }
        }
        if checkpoint(callback, last.bytes, DownloadPhase::Publishing).is_break() {
            return Err(Error::DownloadCancelled);
        }
        for (stage, path) in stages.iter().zip(&retained) {
            place(&stage.disk, path)?;
        }
        place(&last.disk, &last.path)?;
        // Upstream deletes the processed original unless it is kept.
        if let Some(existing) = existing
            && existing != last.path
            && !retained.iter().any(|path| path == existing)
            && fs::symlink_metadata(existing).is_ok()
            && !keep
        {
            fs::remove_file(existing)?;
        }
        Ok(ProcessedDownload {
            path: last.path.clone(),
            bytes: last.bytes,
            kept_video: retained.first().cloned(),
            kept_files: retained,
            warnings,
            errors,
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
                    && let Err(preserve) = place(&stage.disk, &path)
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
        Ok(result) => {
            let mut failures = Vec::new();
            for (path, metadata) in delete_subtitles {
                let remove = (|| -> Result<()> {
                    if !same_subtitle(&metadata, &subtitle_metadata(&path)?) {
                        return Err(Error::Postprocess(
                            "embedded subtitle changed; source retained".into(),
                        ));
                    }
                    fs::remove_file(path)?;
                    Ok(())
                })();
                if let Err(error) = remove {
                    failures.push(error.to_string());
                }
            }
            if !failures.is_empty() {
                return Err(Error::Postprocess(format!(
                    "media published, but embedded subtitle cleanup failed: {}",
                    failures.join("; ")
                )));
            }
            Ok(result)
        }
    }
}

fn subtitle_metadata(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::Postprocess(
            "subtitle embedding requires regular local sidecars".into(),
        ));
    }
    Ok(metadata)
}
fn same_subtitle(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    let same = before.len() == after.len() && before.modified().ok() == after.modified().ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same && before.dev() == after.dev() && before.ino() == after.ino()
    }
    #[cfg(not(unix))]
    {
        same
    }
}
