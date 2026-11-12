//! Portable subprocess fixture for end-to-end audio publication tests.
use std::{fs, io::Write, path::PathBuf};
fn main() {
    let executable = std::env::current_exe().unwrap();
    let name = executable.file_stem().unwrap().to_str().unwrap();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(executable.with_extension("log"))
        .unwrap();
    writeln!(log, "{}", args.join("\n")).unwrap();
    let probe = name.starts_with("probe");
    if args == ["-version"] {
        println!(
            "{} version fixture",
            if probe { "ffprobe" } else { "ffmpeg" }
        );
        return;
    }
    assert!(
        args.windows(2)
            .any(|pair| pair == ["-protocol_whitelist", "file"])
    );
    let input = PathBuf::from(&args[args.iter().position(|arg| arg == "-i").unwrap() + 1]);
    assert!(input.is_absolute());
    assert!(!fs::read(&input).unwrap().is_empty());
    if probe {
        let codec = name.strip_prefix("probe-").unwrap();
        println!(
            "{{\"streams\":[{{\"codec_type\":\"video\",\"codec_name\":\"h264\"}},{{\"codec_type\":\"audio\",\"codec_name\":\"{codec}\"}}]}}"
        );
    } else if args.last() == Some(&input.display().to_string()) {
        // `ffmpeg -i INPUT` without an output: the stream list, exit 1.
        eprintln!(
            "Input #0, ogg, from '{}':\n  Stream #0:0: Audio: opus, 48000 Hz, stereo, fltp\n\
             At least one output file must be specified",
            input.display()
        );
        std::process::exit(1);
    } else {
        let output = PathBuf::from(args.last().unwrap());
        let inputs: Vec<_> = args.windows(2).filter(|pair| pair[0] == "-i").collect();
        if inputs.len() == 2 {
            for pair in inputs {
                assert_eq!(fs::read(&pair[1]).unwrap(), b"source audio");
            }
            fs::write(output, b"source audio").unwrap();
        } else {
            fs::write(output, b"converted audio").unwrap();
        }
        if name == "ffmpeg-fail" {
            std::process::exit(1);
        }
    }
}
