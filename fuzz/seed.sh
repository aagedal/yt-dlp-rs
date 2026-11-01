#!/usr/bin/env bash
# Seed fuzz/corpus/TARGET from the test fixtures, so runs start from real
# manifests and files instead of empty input. Safe to rerun.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
fixtures="$root/tests/fixtures"
corpus="$root/fuzz/corpus"

seed() {
    target=$1
    shift
    mkdir -p "$corpus/$target"
    find "$fixtures" -type f \( "$@" \) -size -1M -print0 |
        while IFS= read -r -d '' file; do
            cp "$file" "$corpus/$target/$(printf '%s' "$file" | shasum | cut -c1-16)"
        done
}

seed hls -name '*.m3u8'
seed dash -name '*.mpd'
seed media_index -name '*.mp4' -o -name '*.webm' -o -name '*.m4a'
seed js_to_json -name '*.js' -o -name '*.html'
seed devalue -path '*devalue*' -o -name '*.json'
seed ttml -name '*.ttml' -o -name '*.xml'
seed cookie_file -name 'cookies*.txt' -o -path '*/cookies/*'
seed sqlite -name '*.sqlite' -o -name 'Cookies' -o -name 'cookies.sqlite'
seed f4m -name '*.abst' -o -name '*.f4m'
seed ism -name '*.ismv' -o -name '*.isma' -o -path '*ism*'
seed info_json -name '*.info.json'
seed netrc -name '*netrc*'
seed html -name '*.html' -o -name '*.htm'
seed dates -name '*.json' -size -4k
seed cover_art -name '*.flac' -o -name '*.ogg' -o -name '*.opus' -o -name '*.m4a' -o -name '*.mp4' -o -name '*.png' -o -name '*.jpg'
