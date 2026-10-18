# Original HLS test media

Generated offline with FFmpeg 9.0.2 from color and sine sources; no third-party
media was used. From the repository root, generate each set with:

```sh
ffmpeg -nostdin -hide_banner -loglevel error -f lavfi -i color=c=blue:s=32x32:r=10:d=1.8 -f lavfi -i sine=frequency=440:sample_rate=48000:duration=1.8 -c:v libx264 -pix_fmt yuv420p -g 6 -threads 1 -c:a aac -b:a 32k -hls_time 0.6 -hls_list_size 0 -hls_segment_filename tests/fixtures/hls/ts/segment-%d.ts -y tests/fixtures/hls/ts/index.m3u8
ffmpeg -nostdin -hide_banner -loglevel error -f lavfi -i color=c=blue:s=32x32:r=10:d=1.8 -f lavfi -i sine=frequency=440:sample_rate=48000:duration=1.8 -c:v libx264 -pix_fmt yuv420p -g 6 -threads 1 -c:a aac -b:a 32k -hls_time 0.6 -hls_list_size 0 -hls_segment_type fmp4 -hls_segment_filename tests/fixtures/hls/fmp4/segment-%d.m4s -y tests/fixtures/hls/fmp4/index.m3u8
```

`range/combined.mp4` concatenates fMP4 `init.mp4` followed by the three media
fragments. Its playlist describes the exact initialization and segment byte
lengths: an explicit first media offset followed by implicit continuation offsets.
When regenerating, derive those ranges from the resulting file lengths.

Required tests verify exact assembled bytes. With `YT_DLP_RS_TEST_FFPROBE` set,
the same test also confirms H.264 video and AAC audio in TS, fMP4 and byte-range
output. No live sites or runtime Python fallback are involved.
