//! HlsFD's `extra_param_to_segment_url` and `extra_param_to_key_url`:
//! `parse_qs`'d format fields merged with `update_url_query` into every
//! fragment URL (initialization included) and every AES-128 key URL, the
//! key URLs falling back to the segment query.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{Arc, Mutex},
    thread,
};

const KEY: [u8; 16] = [0x43; 16];
const IV: [u8; 16] = [0; 16];
const PLAIN: [u8; 188] = [0x47; 188];
const ENCRYPTED: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-KEY:METHOD=AES-128,URI=\"key?a=0\",IV=0x00000000000000000000000000000000\n#EXTINF:1,\ns0.ts?keep=1&t=0\n#EXT-X-KEY:METHOD=AES-128,URI=\"key2\",IV=0x00000000000000000000000000000000\n#EXTINF:1,\ns1.ts\n#EXT-X-ENDLIST\n";
const INIT: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\ns0.m4s\n#EXT-X-ENDLIST\n";

fn encrypt(plain: &[u8]) -> Vec<u8> {
    use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
    let mut buffer = vec![0; plain.len() + 16];
    buffer[..plain.len()].copy_from_slice(plain);
    cbc::Encryptor::<aes::Aes128>::new((&KEY).into(), (&IV).into())
        .encrypt_padded::<Pkcs7>(&mut buffer, plain.len())
        .unwrap()
        .to_vec()
}

/// Serves the playlists, segments and keys (the right key only when the
/// request carries `k=2` or `t=1`); records request targets.
fn serve() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let record = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let target = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            record.lock().unwrap().push(target.clone());
            let (path, query) = target.split_once('?').unwrap_or((&target, ""));
            let authorized = query.split('&').any(|pair| pair == "k=2" || pair == "t=1");
            let (mime, body): (&str, Vec<u8>) = match path {
                "/encrypted.m3u8" => ("application/vnd.apple.mpegurl", ENCRYPTED.into()),
                "/init.m3u8" => ("application/vnd.apple.mpegurl", INIT.into()),
                "/s0.ts" | "/s1.ts" => ("video/mp2t", encrypt(&PLAIN)),
                "/key" | "/key2" if authorized => ("application/octet-stream", KEY.into()),
                "/init.mp4" => ("video/mp4", b"init".to_vec()),
                "/s0.m4s" => ("video/mp4", b"media".to_vec()),
                _ => ("application/octet-stream", vec![0; 16]),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    (origin, requests)
}

/// Downloads a `--load-info-json` m3u8_native format carrying `extra`.
fn run(directory: &Path, origin: &str, playlist: &str, extra: serde_json::Value) -> Output {
    let mut info = serde_json::json!({
        "id": "x",
        "title": "x",
        "ext": "mp4",
        "format_id": "hls",
        "url": format!("{origin}/{playlist}"),
        "protocol": "m3u8_native",
        "webpage_url": format!("{origin}/"),
        "extractor": "generic",
        "extractor_key": "Generic",
    });
    info.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    std::fs::write(directory.join("in.info.json"), info.to_string()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "--fixup", "never"])
        .args(["-o", "o.%(ext)s", "--load-info-json", "in.info.json"])
        .current_dir(directory)
        .env("PATH", "")
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn segment_and_key_queries_update_fragment_and_key_urls() {
    let (origin, requests) = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &origin,
        "encrypted.m3u8",
        serde_json::json!({
            "extra_param_to_segment_url": "t=1&keep=2&x=a b~*&blank=",
            "extra_param_to_key_url": "k=2",
        }),
    );
    assert_success(&output);
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        PLAIN.repeat(2)
    );
    let requests = requests.lock().unwrap();
    assert_eq!(
        *requests,
        [
            "/encrypted.m3u8",
            "/key?a=0&k=2",
            // An existing parameter keeps its place and takes the new value;
            // `parse_qs` drops blank values.
            "/s0.ts?keep=2&t=1&x=a+b~%2A",
            "/key2?k=2",
            "/s1.ts?t=1&keep=2&x=a+b~%2A",
        ]
    );
}

#[test]
fn key_urls_fall_back_to_the_segment_query() {
    let (origin, requests) = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &origin,
        "encrypted.m3u8",
        serde_json::json!({"extra_param_to_segment_url": "t=1"}),
    );
    assert_success(&output);
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        PLAIN.repeat(2)
    );
    let requests = requests.lock().unwrap();
    assert_eq!(
        *requests,
        [
            "/encrypted.m3u8",
            "/key?a=0&t=1",
            "/s0.ts?keep=1&t=1",
            "/key2?t=1",
            "/s1.ts?t=1",
        ]
    );
}

#[test]
fn initialization_fragments_carry_the_segment_query() {
    let (origin, requests) = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        directory.path(),
        &origin,
        "init.m3u8",
        serde_json::json!({"extra_param_to_segment_url": "t=1"}),
    );
    assert_success(&output);
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"initmedia"
    );
    let requests = requests.lock().unwrap();
    assert_eq!(*requests, ["/init.m3u8", "/init.mp4?t=1", "/s0.m4s?t=1"]);
}
