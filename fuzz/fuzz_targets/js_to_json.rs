#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| yt_dlp_rs::fuzz::js_to_json(data));
