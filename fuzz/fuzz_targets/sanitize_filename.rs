#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| yt_dlp_rs::fuzz::sanitize_filename(data));
