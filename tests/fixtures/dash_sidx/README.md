# Single-file DASH representation with a global SIDX

`video.mp4` is a synthetic 8-second 160x120 test pattern (FFmpeg's `testsrc`),
H.264 with one-second fragments and one global `sidx` box, generated with:

```sh
ffmpeg -f lavfi -i testsrc=duration=8:size=160x120:rate=10 -f lavfi -i sine=duration=8 \
  -c:v libx264 -g 10 -keyint_min 10 -c:a aac -f dash -single_file 1 -global_sidx 1 \
  -seg_duration 1 out.mpd
```

(`out-stream0.mp4`, renamed). Its boxes: `ftyp` 0-31, `moov` 32-818, `sidx`
819-954, then eight `moof`/`mdat` pairs through byte 32582. `tests/dash_sidx.rs`
serves it behind a hand-written `SegmentBase` MPD (`indexRange="819-954"`,
`Initialization range="0-818"`).
