#!/bin/sh
# Writes THIRD-PARTY-NOTICES.txt for the release binaries (every one is built
# with `impersonate`) to $1: the Rust dependencies' licenses from cargo-about,
# then the notices bundled into the embedded YouTube challenge solver.
set -eu
out=${1:-THIRD-PARTY-NOTICES.txt}
case $out in /*) ;; *) out=$PWD/$out ;; esac
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
cargo about generate --locked --fail --features impersonate \
    -c tools/notices/about.toml -o "$out" tools/notices/notices.hbs
{
    printf '\n%s\n' '================================================================================'
    printf '%s\n\n' 'yt-dlp/ejs (vendor/ejs), embedded with its bundled dependencies'
    awk 'NR == 1, /^ \*\/$/' vendor/ejs/yt.solver.lib.js
} >> "$out"
