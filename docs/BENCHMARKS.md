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

## Results (2026-11-03)

Machine: Apple M5 Pro (18 cores), 64 GB, macOS 27.2, APFS SSD. yt-dlp-rs
0.8.0-beta.1 plus the changes below (release build, Rust 1.99.1); upstream
2026.10.26 (`f264a6cb`, the pinned baseline) on CPython 3.15.2, run from
source. Five runs per case. The full data is in
[`benchmarks/2026-11-03-apple-m5-pro.json`](benchmarks/2026-11-03-apple-m5-pro.json)
and [`benchmarks/2026-11-03-apple-m5-pro-400mib.json`](benchmarks/2026-11-03-apple-m5-pro-400mib.json);
the 2026-11-01 run before these changes is in
[`benchmarks/2026-11-01-apple-m5-pro.json`](benchmarks/2026-11-01-apple-m5-pro.json).

### Startup and extraction

With the URL prefilter below (run separately on 2026-11-03,
[`benchmarks/2026-11-03-apple-m5-pro-url-prefilter.json`](benchmarks/2026-11-03-apple-m5-pro-url-prefilter.json);
upstream's times were about 15% above the earlier run's on the same machine):

| Case | yt-dlp-rs | upstream | yt-dlp-rs RSS | upstream RSS |
|---|---:|---:|---:|---:|
| `--version` | 0.007 s | 0.094 s | 6.1 MiB | 38.3 MiB |
| `--simulate` direct URL | 0.015 s | 0.536 s | 23.7 MiB | 74.7 MiB |
| YouTube `-J` (`--no-js-runtimes`) | 0.024 s | 0.451 s | 31.9 MiB | 69.3 MiB |
| Instagram `-J` | 0.015 s | 0.474 s | 19.9 MiB | 70.1 MiB |
| TikTok `-J` | 0.022 s | 0.543 s | 20.4 MiB | 73.7 MiB |
| Vimeo `-J` | 0.021 s | 0.543 s | 21.2 MiB | 73.9 MiB |
| Dailymotion `-J` | 0.019 s | 0.461 s | 20.7 MiB | 69.8 MiB |
| Bandcamp `-J` | 0.017 s | 0.457 s | 22.0 MiB | 69.5 MiB |
| SoundCloud `-J` | 0.023 s | 0.536 s | 22.0 MiB | 73.7 MiB |
| X `-J` | 0.022 s | 0.542 s | 21.7 MiB | 73.8 MiB |
| Facebook `-J` | 0.015 s | 0.466 s | 21.3 MiB | 69.8 MiB |
| Bluesky `-J` | 0.017 s | 0.457 s | 20.8 MiB | 69.5 MiB |

The port starts and extracts these fixtures 19–36× faster in wall time;
upstream's time is mostly interpreter start and imports.

Matching a URL means trying every extractor in upstream's order, and each
one compiled its URL pattern on first use, so a URL no site extractor takes
compiled all of them (YouTube's extractors come late, so a YouTube URL
compiled nearly all). On 2026-11-01 that cost about 80 MiB (101 MiB peak for
the direct-URL `--simulate` case; `--use-extractors generic`, 18 MiB) and
most of the wall time. Two changes removed it:

- Most of the memory (41 MiB, measured with `MallocStackLogging`) was the
  `regex` crate's one-pass DFA, which it builds for every pattern with a
  capture group but only uses for anchored searches. The crate is built
  without its `perf-onepass` feature: anchored captures use the bounded
  backtracker, which is as fast on URL-sized input. That alone cut 17–37 MiB
  per case (direct URL 101 → 64 MiB, YouTube 91 → 58 MiB).
- The patterns dispatch tries (about 930) are `UrlRegex`es
  (`src/extractor/mod.rs`): each has a set of literals of which every
  match contains one, and compiles only when the haystack contains one of
  them, so a YouTube URL compiles only a few. The sets are derived from the
  patterns' syntax trees by `tests/url_literals.rs` and saved, keyed by
  pattern hash, in the generated `src/extractor/url_literals.rs`, so
  finding them costs nothing at run time; the test fails when a pattern
  lacks an entry and checks the sets against every fixture URL. Direct URL
  0.068 → 0.015 s and 64 → 24 MiB; YouTube `-J` 0.057 → 0.024 s and 58 → 32
  MiB.

### HLS and DASH, 100 × 256 KiB (25 MiB)

| Case | yt-dlp-rs | upstream | yt-dlp-rs RSS | upstream RSS |
|---|---:|---:|---:|---:|
| HLS `-N 1`, no latency | 0.111 s | 0.696 s | 65.4 MiB | 75.5 MiB |
| HLS `-N 4`, no latency | 0.089 s | 0.655 s | 76.0 MiB | 75.6 MiB |
| HLS `-N 8`, no latency | 0.090 s | 0.659 s | 80.8 MiB | 77.2 MiB |
| HLS `-N 1`, 50 ms | 6.061 s | 7.333 s | 67.1 MiB | 75.9 MiB |
| HLS `-N 4`, 50 ms | 1.711 s | 2.361 s | 72.8 MiB | 75.8 MiB |
| HLS `-N 8`, 50 ms | 1.020 s | 1.574 s | 76.2 MiB | 76.5 MiB |
| DASH `-N 1`, no latency | 0.109 s | 0.693 s | 66.1 MiB | 75.4 MiB |
| DASH `-N 4`, no latency | 0.090 s | 0.660 s | 74.5 MiB | 75.8 MiB |
| DASH `-N 8`, no latency | 0.090 s | 0.661 s | 78.9 MiB | 77.2 MiB |
| DASH `-N 1`, 50 ms | 6.023 s | 7.208 s | 68.0 MiB | 76.0 MiB |
| DASH `-N 4`, 50 ms | 1.647 s | 2.200 s | 74.2 MiB | 75.9 MiB |
| DASH `-N 8`, 50 ms | 0.913 s | 1.397 s | 81.3 MiB | 77.0 MiB |

### HLS and DASH, 400 × 1 MiB (400 MiB), no latency

| Case | yt-dlp-rs | upstream | yt-dlp-rs RSS | upstream RSS |
|---|---:|---:|---:|---:|
| HLS `-N 1` | 0.302 s | 1.590 s | 67.8 MiB | 75.4 MiB |
| HLS `-N 4` | 0.467 s | 1.372 s | 118.8 MiB | 81.2 MiB |
| HLS `-N 8` | 0.468 s | 1.369 s | 138.9 MiB | 83.3 MiB |
| DASH `-N 1` | 0.304 s | 1.739 s | 68.4 MiB | 75.4 MiB |
| DASH `-N 4` | 0.460 s | 1.421 s | 121.7 MiB | 79.8 MiB |
| DASH `-N 8` | 0.479 s | 1.333 s | 135.2 MiB | 82.8 MiB |

With 50 ms per request, both programs are bound by request count, and `-N`
helps both about equally: `-N 4` is 3.5× (port) and 3.1–3.3× (upstream)
faster than `-N 1`, `-N 8` is 5.9–6.6× and 4.7–5.2×. The 50 ms timings of
both programs are 5–10% above 2026-11-01's on the same machine, so compare
them within one run only.

`-N 1` writes fragments straight into the output file. On 2026-11-01 the
`-N > 1` path stored every fragment in a private temporary file and then
copied it into place, so every byte was written twice and `-N 4/8` took
1.8–2.1× as long as `-N 1` on 400 MiB. Concurrent workers now fill
in-memory spools that the assembling thread writes once. A download's
spools share 32 MiB (`SPOOL_MEMORY` in `src/fragment.rs`, counted before
each allocation); a fragment that would exceed it moves to a private
file as before. On 25 MiB, `-N 4/8` is now faster than `-N 1`. On 400 MiB
the localhost server outruns the single writer, the backlog fills the
budget and later fragments spill, so `-N 4/8` take 1.5× as long as `-N 1`
(0.68 s before; budgets of 0, 16, 64 and 128 MiB gave 0.67, 0.52, 0.39
and 0.28 s at `-N 4`, with peak RSS rising from 83 to 202 MiB).

Since 2026-11-11 the workers stop running ahead of the writer instead
(`Window` in `src/fragment.rs`): once the next fragment to append has
arrived, so the backlog is the disk's and not a slow server's, a fragment
starts only while the queued and in-flight fragments fit in the budget.
While the next fragment is still in flight (a slow response, a retry), the
others keep going and spill as before, like upstream's workers. Spools
reserve a response's announced length once, and released buffers are
reused, instead of fragmenting the heap with a new 1 MiB buffer each.
Measured on a loaded M5 Pro (another build and an antivirus scanner
running, so absolute times are not comparable with the tables above), 400
× 1 MiB from a local server: `-N 4/8` went from about 350 spilled
fragments at once to 0–7, system time from 0.86 s to 0.31 s (the same as
`-N 1`), wall time from 3.1–6.8 s to 1.6–1.9 s (`-N 1`: 2.1–4.1 s), and
peak RSS from 82–111 MiB to 72–80 MiB. The 50 ms latency cases did not
change.

Rerun on 2026-11-12 with the machine idle (load about 5), `benchmark.py
--segments 400 --segment-kib 1024 --latencies 0,50 --workers 1,4,8
--runs 5`, medians, the release binary before (187d43f7) and after:

| case | `-N 1` | `-N 4` | `-N 8` |
| --- | --- | --- | --- |
| hls, 0 ms, before | 0.47 s, 30 MiB | 1.63 s, 74 MiB | 1.67 s, 92 MiB |
| hls, 0 ms, after | 0.43 s, 29 MiB | 0.28 s, 69 MiB | 0.30 s, 75 MiB |
| dash, 0 ms, before | 0.37 s, 30 MiB | 1.46 s, 75 MiB | 1.50 s, 91 MiB |
| dash, 0 ms, after | 0.44 s, 29 MiB | 0.27 s, 68 MiB | 0.28 s, 76 MiB |
| hls, 50 ms, before | 22.7 s, 35 MiB | 5.69 s, 65 MiB | 2.97 s, 85 MiB |
| hls, 50 ms, after | 22.8 s, 31 MiB | 5.69 s, 47 MiB | 2.95 s, 60 MiB |
| dash, 50 ms, before | 22.5 s, 37 MiB | 5.59 s, 74 MiB | 2.85 s, 86 MiB |
| dash, 50 ms, after | 22.7 s, 32 MiB | 5.62 s, 45 MiB | 2.89 s, 61 MiB |

On localhost `-N 4/8` are now 5–6× faster than before and 1.5× faster than
`-N 1`, instead of 3.5× slower. With latency the times are unchanged and
peak RSS is 20–40% lower. `-N 1` differences are run-to-run noise.

### Default `-N`

The port keeps upstream's default of `-N 1` (`concurrent_fragment_downloads`
1). The measurements favor `-N 4` only when every request has real
latency. The default is a compatibility surface: some sites rate-limit or
throttle parallel fragment requests, and upstream's choice is what users
and scripts expect. The gain is
available on request: Media Converter can pass `-N 4` for HLS/DASH
downloads, as yt-dlp users do. Revisit if canary runs with `-N 4` show
no throttling.

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
