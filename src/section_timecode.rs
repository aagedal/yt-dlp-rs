//! `--section-timecode` (port extension): a partial download's first frame
//! gets the timecode it has in the whole video (frame N of the whole file is
//! `N` formatted at the nominal rate, non-drop-frame), so clips line up with
//! a full download in editing and logging tools. FFmpeg writes it with
//! `-timecode`: a `tmcd` track in MP4/MOV, a `TIMECODE` tag elsewhere that a
//! later remux to MP4/MOV turns into the track. Other final containers and
//! audio-only sections are skipped with a warning.
use crate::{VideoInfo, ffmpeg_fd::LocalInput, info::PyNumber};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// The containers whose timecode track editors read.
const TRACK_CONTAINERS: [&str; 3] = ["mp4", "mov", "m4v"];

/// Settings for one download.
pub(crate) struct Request {
    pub ffprobe: Option<PathBuf>,
    /// The extension after `--remux-video`/`--recode-video`.
    pub final_ext: String,
}

/// One input as FFmpeg reads it: a local segment file or the format URL.
pub(crate) struct Input<'a> {
    pub info: &'a VideoInfo,
    pub local: Option<&'a LocalInput>,
    /// `-headers` for a URL input.
    pub headers: Option<String>,
}

/// What ffprobe reports about the video stream.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Probe {
    rate: f64,
    /// The video stream's and the file's start times.
    video_start: f64,
    file_start: f64,
    /// The first packet after seeking, when a seek was requested.
    first: Option<f64>,
}

fn warn(reason: &str) {
    crate::screen::warning(&format!("Not writing a section timecode: {reason}"));
}

fn is_video(info: &VideoInfo) -> bool {
    info.vcodec.as_deref() != Some("none")
}

fn seconds(value: Option<PyNumber>) -> f64 {
    value.map_or(0.0, |value| value.as_f64())
}

/// A rational frame rate (`30000/1001`).
fn rate(text: &str) -> Option<f64> {
    let (numerator, denominator) = text.split_once('/').unwrap_or((text, "1"));
    let rate = numerator.parse::<f64>().ok()? / denominator.parse::<f64>().ok()?;
    (rate.is_finite() && rate > 0.0).then_some(rate)
}

/// `HH:MM:SS:FF` for frame `frames` at the nominal rate of `rate`.
pub(crate) fn format(frames: u64, rate: f64) -> String {
    let nominal = (rate.round() as u64).max(1);
    let seconds = frames / nominal;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
        frames % nominal
    )
}

/// The first shown frame's index in the whole video. `start` is the
/// section start relative to the file start, `edit_list` whether the output
/// hides the pre-roll before it (MP4/MOV); otherwise the output starts at
/// the keyframe FFmpeg seeked to.
fn first_frame(probe: &Probe, start: f64, edit_list: bool) -> u64 {
    let offset = probe.file_start - probe.video_start;
    let frames = if edit_list || start == 0.0 {
        // The first frame at or after the start.
        ((start + offset) * probe.rate - 1e-6).ceil()
    } else {
        let first = probe.first.unwrap_or(probe.video_start);
        ((first - probe.video_start) * probe.rate).round()
    };
    frames.max(0.0) as u64
}

/// ffprobe's view of `input`'s first video stream; `seek` (absolute
/// seconds) also reads the first packet after seeking there like FFmpeg.
fn probe(ffprobe: &Path, input: &Input<'_>, seek: Option<f64>) -> Option<Probe> {
    let mut args: Vec<OsString> = ["-v", "error", "-select_streams", "v:0", "-of", "json"]
        .map(OsString::from)
        .into();
    let mut entries =
        String::from("format=start_time:stream=avg_frame_rate,r_frame_rate,start_time");
    if let Some(seek) = seek {
        args.extend([
            "-read_intervals".into(),
            format!("{}%+#1", crate::screen::py_float(seek)).into(),
        ]);
        entries.push_str(":packet=pts_time");
    }
    args.extend(["-show_entries".into(), entries.into()]);
    match input.local {
        Some(local) => {
            let mut path = OsString::from("file:");
            path.push(&local.path);
            args.push(path);
        }
        None => {
            if let Some(headers) = &input.headers {
                args.extend(["-headers".into(), headers.into()]);
            }
            args.extend(["-rw_timeout".into(), "30000000".into()]);
            args.push(input.info.url.as_str().into());
        }
    }
    crate::screen::debug_command(ffprobe, &args);
    let output = Command::new(ffprobe)
        .args(&args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let stream = value["streams"].get(0)?;
    let number = |value: &serde_json::Value| value.as_str()?.parse::<f64>().ok();
    let rate = ["avg_frame_rate", "r_frame_rate"]
        .iter()
        .find_map(|key| rate(stream[*key].as_str()?))?;
    let mut probe = Probe {
        rate,
        video_start: number(&stream["start_time"]).unwrap_or(0.0),
        file_start: number(&value["format"]["start_time"]).unwrap_or(0.0),
        first: seek.and_then(|_| number(&value["packets"].get(0)?["pts_time"])),
    };
    // A local segment file starts later; its original timeline starts at
    // the recorded start time (each YouTube input has one stream).
    if let Some(local) = input.local {
        probe.video_start = local.start_time;
        probe.file_start = local.start_time;
    }
    Some(probe)
}

/// The `-timecode` value for a section download, or `None` (warned) when
/// it cannot be written. `inputs` are FFmpeg's inputs in order; `muxer` is
/// the cut's output format.
pub(crate) fn timecode(
    request: &Request,
    info: &VideoInfo,
    inputs: &[Input<'_>],
    muxer: &str,
) -> Option<String> {
    let Some(input) = inputs.iter().find(|input| is_video(input.info)) else {
        warn("the section has no video stream");
        return None;
    };
    let final_ext = request.final_ext.to_ascii_lowercase();
    if !TRACK_CONTAINERS.contains(&final_ext.as_str()) {
        warn(&format!(
            "{final_ext} files cannot carry a timecode track; remux the video to mp4 or mov (e.g. --remux-video mp4)"
        ));
        return None;
    }
    let Some(ffprobe) = request.ffprobe.as_deref().filter(|path| path.exists()) else {
        warn("ffprobe was not found (--ffprobe-location)");
        return None;
    };
    let start = seconds(info.section_start);
    let edit_list = matches!(muxer, "mp4" | "mov" | "ipod");
    let mut probed = probe(ffprobe, input, None);
    // Without edit lists the output starts at the keyframe FFmpeg seeks to
    // (`-ss` is relative to the file's start).
    if let Some(found) = probed.filter(|_| !edit_list && start != 0.0) {
        probed = probe(ffprobe, input, Some(start + found.file_start)).map(|mut probe| {
            probe.video_start = found.video_start;
            probe.file_start = found.file_start;
            probe
        });
    }
    let Some(probe) = probed else {
        warn("ffprobe could not read the video stream's frame rate");
        return None;
    };
    let timecode = format(first_frame(&probe, start, edit_list), probe.rate);
    crate::screen::screen(&format!(
        "[download] Section timecode: {timecode} ({} fps)",
        crate::screen::py_float((probe.rate * 1000.0).round() / 1000.0)
    ));
    Some(timecode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecodes_count_frames_at_the_nominal_rate() {
        assert_eq!(format(0, 25.0), "00:00:00:00");
        assert_eq!(format(533, 25.0), "00:00:21:08");
        // 29.97 fps is counted at 30 frames per timecode second (NDF).
        assert_eq!(format(3600 * 30 + 29, 30000.0 / 1001.0), "01:00:00:29");
        assert_eq!(format(25 * 86_400 + 1, 25.0), "00:00:00:01");
        assert_eq!(rate("30000/1001"), Some(30000.0 / 1001.0));
        assert_eq!(rate("25"), Some(25.0));
        assert_eq!(rate("0/0"), None);
    }

    #[test]
    fn the_first_shown_frame_follows_the_output_container() {
        let probe = Probe {
            rate: 25.0,
            video_start: 0.08,
            file_start: 0.08,
            first: Some(20.08),
        };
        // MP4 edit lists start at the first frame at or after 21.3 s.
        assert_eq!(first_frame(&probe, 21.3, true), 533);
        assert_eq!(first_frame(&probe, 21.32, true), 533);
        assert_eq!(first_frame(&probe, 0.0, false), 0);
        // Elsewhere the output starts at the keyframe sought to (20.08 s).
        assert_eq!(first_frame(&probe, 21.3, false), 500);
        // A muxed file whose audio starts before its video.
        let muxed = Probe {
            video_start: 0.1,
            file_start: 0.0,
            ..probe
        };
        assert_eq!(first_frame(&muxed, 4.0, true), 98);
    }
}
