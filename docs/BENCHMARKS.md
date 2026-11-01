# Benchmarks

`tools/benchmark.py` runs the release binary and the pinned upstream against
local servers only, and reports the median of repeated runs (after one
warm-up run) plus peak RSS from `/usr/bin/time` (`-l` on macOS, `-v` on
Linux). It is a development tool, not a test: timings depend on the machine,
so nothing here gates CI. Only deterministic behavior (such as the lazy
playlist request counts in `tests/lazy_playlist.rs`) is in `cargo test`.

```sh
cargo build --release --locked
python3 tools/benchmark.py --upstream .upstream/yt-dlp --runs 5 --output result.json
```

What it measures:

- **startup**: `--version`, and `--simulate` of a direct media URL on a
  local server.
- **extract**: `-J` of saved site fixtures (one case each for the Media
  Converter sites that have a `-J` fixture), served by
  `tools/site_proxy.py` as in `tools/capture_sites.py`.
- **hls**, **dash**: downloading a generated VOD (an MPEG-TS playlist, or an
  MPD `SegmentTemplate` with an init segment) with `-N 1/4/8`, from a server
  answering immediately and from one that waits 50 ms before every response.
  Each run must leave exactly the expected bytes on disk.

## Results (2026-11-01)

Machine: Apple M5 Pro (18 cores), 64 GB, macOS 27.2, APFS SSD. yt-dlp-rs
0.7.0-beta.2 (this branch, release build, Rust 1.99.1); upstream 2026.10.26
(`f264a6cb`, the pinned baseline) on CPython 3.15.2, run from source.
Five runs per case. The full data is in
[`benchmarks/2026-11-01-apple-m5-pro.json`](benchmarks/2026-11-01-apple-m5-pro.json).

### Startup and extraction

| Case | yt-dlp-rs | upstream | yt-dlp-rs RSS | upstream RSS |
|---|---:|---:|---:|---:|
| `--version` | 0.007 s | 0.127 s | 5.7 MiB | 38.5 MiB |
| `--simulate` direct URL | 0.077 s | 0.502 s | 101.1 MiB | 74.7 MiB |
| YouTube `-J` (`--no-js-runtimes`) | 0.065 s | 0.407 s | 91.5 MiB | 69.3 MiB |
| Instagram `-J` | 0.034 s | 0.434 s | 48.1 MiB | 70.1 MiB |
| TikTok `-J` | 0.038 s | 0.504 s | 45.9 MiB | 73.8 MiB |
| Vimeo `-J` | 0.040 s | 0.523 s | 50.1 MiB | 73.9 MiB |
| Dailymotion `-J` | 0.037 s | 0.431 s | 49.3 MiB | 69.7 MiB |
| Bandcamp `-J` | 0.038 s | 0.427 s | 53.0 MiB | 69.5 MiB |
| SoundCloud `-J` | 0.044 s | 0.493 s | 51.9 MiB | 73.8 MiB |
| X `-J` | 0.043 s | 0.518 s | 45.3 MiB | 73.9 MiB |
| Facebook `-J` | 0.034 s | 0.415 s | 50.0 MiB | 69.9 MiB |
| Bluesky `-J` | 0.041 s | 0.398 s | 56.0 MiB | 69.4 MiB |

The port starts and extracts these fixtures 6–18× faster in wall time;
upstream's time is mostly interpreter start and imports.

Peak memory is not uniformly lower. Matching a URL against every extractor
compiles all of their URL patterns, which costs about 80 MiB here
(`--use-extractors generic` brings the direct-URL `--simulate` case down to
18 MiB). The YouTube case is higher still. Compiling patterns lazily or
sharing them is a follow-up.

### HLS and DASH, 100 × 256 KiB (25 MiB)

| Case | yt-dlp-rs | upstream | yt-dlp-rs RSS | upstream RSS |
|---|---:|---:|---:|---:|
| HLS `-N 1`, no latency | 0.122 s | 0.733 s | 102.3 MiB | 75.3 MiB |
| HLS `-N 4`, no latency | 0.162 s | 0.685 s | 105.3 MiB | 75.8 MiB |
| HLS `-N 8`, no latency | 0.162 s | 0.732 s | 110.2 MiB | 77.4 MiB |
| HLS `-N 1`, 50 ms | 5.717 s | 6.669 s | 105.1 MiB | 76.2 MiB |
| HLS `-N 4`, 50 ms | 1.647 s | 2.223 s | 107.9 MiB | 76.0 MiB |
| HLS `-N 8`, 50 ms | 0.964 s | 1.563 s | 110.4 MiB | 77.3 MiB |
| DASH `-N 1`, no latency | 0.127 s | 0.774 s | 104.3 MiB | 75.4 MiB |
| DASH `-N 4`, no latency | 0.173 s | 0.767 s | 107.5 MiB | 76.0 MiB |
| DASH `-N 8`, no latency | 0.163 s | 0.760 s | 110.5 MiB | 77.2 MiB |
| DASH `-N 1`, 50 ms | 5.640 s | 6.527 s | 105.2 MiB | 75.8 MiB |
| DASH `-N 4`, 50 ms | 1.575 s | 2.123 s | 108.5 MiB | 75.8 MiB |
| DASH `-N 8`, 50 ms | 0.869 s | 1.375 s | 110.1 MiB | 77.1 MiB |

### HLS and DASH, 400 × 1 MiB (400 MiB), no latency

| Case | yt-dlp-rs | upstream | yt-dlp-rs RSS | upstream RSS |
|---|---:|---:|---:|---:|
| HLS `-N 1` | 0.364 s | 1.758 s | 108.9 MiB | 75.6 MiB |
| HLS `-N 4` | 0.666 s | 1.464 s | 120.9 MiB | 80.4 MiB |
| HLS `-N 8` | 0.688 s | 1.540 s | 130.0 MiB | 82.8 MiB |
| DASH `-N 1` | 0.321 s | 1.856 s | 108.2 MiB | 75.4 MiB |
| DASH `-N 4` | 0.673 s | 1.522 s | 121.5 MiB | 81.5 MiB |
| DASH `-N 8` | 0.690 s | 1.520 s | 129.8 MiB | 86.2 MiB |

With 50 ms per request, both programs are bound by request count, and `-N`
helps both about equally: `-N 4` is 3.5× (port) and 3.0× (upstream) faster
than `-N 1`, `-N 8` is 5.9–6.5× and 4.3–4.7×.

Without latency, the port's `-N 1` writes fragments straight into the
output file. Its `-N > 1` path stores each fragment in a private temporary
file and then copies it into place, so every byte is written twice. On
localhost that makes `-N 4/8` take 1.8–2.1× as long as `-N 1` on 400 MiB
(still about 2.2× faster than upstream, which uses fragment files at every
`-N`). At about
600 MiB/s this only matters when the network is faster than that.
Assembling concurrent fragments without the second copy is a follow-up.

### Default `-N`

The port keeps upstream's default of `-N 1` (`concurrent_fragment_downloads`
1). The measurements favor `-N 4` only when every request has real
latency. The default is a compatibility surface: some sites rate-limit or
throttle parallel fragment requests, and upstream's choice is what users
and scripts expect. On a fast link the port's `-N > 1` is currently slower
(see above). The gain is available on request:
Media Converter can pass `-N 4` for HLS/DASH downloads, as yt-dlp users do.
Revisit if the double write above is removed and canary runs show no
throttling.

### Not covered

- Real networks, TLS and HTTP/2: the media servers are plain HTTP/1.1 on
  127.0.0.1, with a fixed per-request delay standing in for latency (no
  bandwidth limit, loss or jitter). The site fixtures go through the
  fixture proxy's TLS tunnel.
- Upstream with `curl_cffi`, compiled (PyInstaller) builds, or `aria2c`/
  `ffmpeg` downloaders; only upstream's native downloaders are compared.
- YouTube signature/n challenge solving (the case uses `--no-js-runtimes`),
  post-processing, merging, live streams and subtitles.
- Encrypted (AES-128) HLS, byte-range fragments and very large manifests.
- Linux and Windows. The harness reads GNU `time -v` output but was only run
  on macOS.
- CPU time and disk I/O separately from wall time.
