# Fuzz targets

libFuzzer targets for the parsers that read untrusted bytes: HLS, DASH,
SIDX/Cues indexes, `js_to_json`, `devalue`, TTML, Netscape cookie files,
browser cookie databases (SQLite), HDS bootstraps, ISM fragments,
`--load-info-json`, `.netrc`, webpage helpers (meta tags, elements,
JSON-LD), date strings and native cover-art embedding. Entry points live in `src/fuzz.rs`, which
only exists under `--cfg fuzzing`.

```sh
cargo install cargo-fuzz
fuzz/seed.sh                                  # corpus from tests/fixtures
cd fuzz && cargo +nightly fuzz run -O hls -- -max_total_time=600
```

A crash lands in `fuzz/artifacts/TARGET/`. Fix it, save the input under
`tests/fixtures/fuzz/`, and add a regression test next to the parser.
CI runs every target for 30 seconds.
