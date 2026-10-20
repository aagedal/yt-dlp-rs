//! A fixture-serving HTTP(S) proxy for site extractor tests (the Rust twin
//! of `tools/site_proxy.py`): `routes.json` of a `tests/fixtures/sites/SITE`
//! directory answers absolute-URI requests and `CONNECT` tunnels, the latter
//! with the tests' self-signed certificate (clients pass
//! `--no-check-certificate`). Unknown URLs are 404s; nothing leaves the host.
#![allow(dead_code)]
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

const SERVER_CERT: &[u8] = include_bytes!("../fixtures/proxies/tls/server.der");
const SERVER_KEY: &[u8] = include_bytes!("../fixtures/proxies/tls/key.der");

struct Route {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

pub struct SiteProxy {
    pub url: String,
    pub requests: Arc<Mutex<Vec<String>>>,
}

/// The fixture directory of a site.
pub fn site(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sites")
        .join(name)
}

fn load(directory: &Path) -> HashMap<String, Route> {
    let text = std::fs::read_to_string(directory.join("routes.json")).unwrap();
    let data: serde_json::Value = serde_json::from_str(&text).unwrap();
    data["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|route| {
            let body = match route.get("body_file").and_then(serde_json::Value::as_str) {
                Some(file) => std::fs::read(directory.join(file)).unwrap(),
                None => route["body"]
                    .as_str()
                    .unwrap_or_default()
                    .as_bytes()
                    .to_vec(),
            };
            let headers = route["headers"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_owned()))
                .collect();
            (
                route["url"].as_str().unwrap().to_owned(),
                Route {
                    status: route["status"].as_u64().unwrap_or(200) as u16,
                    headers,
                    body,
                },
            )
        })
        .collect()
}

fn read_head(stream: &mut impl Read) -> Option<String> {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return None,
        }
    }
    Some(String::from_utf8_lossy(&head).into_owned())
}

fn respond(stream: &mut impl Write, routes: &HashMap<String, Route>, url: &str) {
    let not_found = Route {
        status: 404,
        headers: vec![("Content-Type".into(), "text/plain".into())],
        body: b"not found".to_vec(),
    };
    let route = routes.get(url).unwrap_or(&not_found);
    let reason = match route.status {
        200 => "OK",
        206 => "Partial Content",
        302 => "Found",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "OK",
    };
    let mut head = format!("HTTP/1.1 {} {reason}\r\n", route.status);
    for (name, value) in &route.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        route.body.len()
    ));
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&route.body);
    let _ = stream.flush();
}

fn handle(
    mut stream: TcpStream,
    routes: &HashMap<String, Route>,
    config: &Arc<ServerConfig>,
    requests: &Mutex<Vec<String>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Some(head) = read_head(&mut stream) else {
        return;
    };
    let mut parts = head.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return;
    };
    if method == "CONNECT" {
        let host = target.strip_suffix(":443").unwrap_or(target).to_owned();
        if stream
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .is_err()
        {
            return;
        }
        let connection = ServerConnection::new(config.clone()).unwrap();
        let mut tls = StreamOwned::new(connection, stream);
        let Some(inner) = read_head(&mut tls) else {
            return;
        };
        let path = inner.split_whitespace().nth(1).unwrap_or("/").to_owned();
        let url = format!("https://{host}{path}");
        requests.lock().unwrap().push(url.clone());
        respond(&mut tls, routes, &url);
        tls.conn.send_close_notify();
        let _ = tls.flush();
    } else {
        requests.lock().unwrap().push(target.to_owned());
        respond(&mut stream, routes, target);
    }
}

impl SiteProxy {
    pub fn start(site_name: &str) -> Self {
        let routes = Arc::new(load(&site(site_name)));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(SERVER_CERT.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(SERVER_KEY.to_vec())),
            )
            .unwrap();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (routes, config, recorded) = (routes.clone(), config.clone(), recorded.clone());
                thread::spawn(move || handle(stream, &routes, &config, &recorded));
            }
        });
        Self { url, requests }
    }
}
