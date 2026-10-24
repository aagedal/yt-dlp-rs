//! Upstream `WebSocketFragmentFD` (`websocket_frag`): an `FFmpegSinkFD`
//! whose connection writes every websocket message (text as UTF-8) into
//! FFmpeg's stdin. Like upstream's `websockets.connect(url,
//! extra_headers=info_dict['http_headers'])`, the handshake sends the
//! format's `http_headers` only (no cookies or proxy). [`connect`] and
//! [`ffmpeg_fd::download_piped`] are the pieces site downloaders (niconico,
//! FC2) build on.
use crate::{
    DownloadOptions, DownloadProgress, Error, Result, VideoInfo,
    ffmpeg_fd::{self, Network, Settings},
};
use std::{io::Write, net::TcpStream, ops::ControlFlow, path::Path, sync::Arc};
use tungstenite::{
    Connector, Message, WebSocket,
    client::IntoClientRequest,
    http::{HeaderName, HeaderValue},
    stream::MaybeTlsStream,
};
use url::Url;

/// An open websocket.
pub(crate) type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

/// The TLS settings for `wss:` (the Mozilla roots, like `websockets`'
/// default context with certifi).
fn connector() -> Connector {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map(|builder| builder.with_root_certificates(roots).with_no_client_auth());
    match config {
        Ok(config) => Connector::Rustls(Arc::new(config)),
        Err(_) => Connector::Plain,
    }
}

/// `websockets.connect(url, extra_headers=headers)`: the opening handshake
/// with the given request headers.
pub(crate) fn connect(url: &Url, headers: &[(String, String)]) -> Result<Socket> {
    let failed = |error: &dyn std::fmt::Display| Error::Reported(format!("{error}"));
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| failed(&error))?;
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| failed(&error))?;
        let value = HeaderValue::from_str(value).map_err(|error| failed(&error))?;
        request.headers_mut().insert(name, value);
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::Reported(format!("{url} has no host")))?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = url.port_or_known_default().unwrap_or(80);
    let stream = TcpStream::connect((host, port))?;
    let connector = (url.scheme() == "wss").then(connector);
    let (socket, _) = tungstenite::client_tls_with_config(request, stream, None, connector)
        .map_err(|error| failed(&error))?;
    Ok(socket)
}

/// `real_connection`'s loop: each message's bytes go to `sink` until the
/// socket closes or the sink's reader is gone (an `OSError`).
pub(crate) fn forward(socket: &mut Socket, sink: &mut dyn Write) -> Result<()> {
    loop {
        let bytes = match socket.read() {
            Ok(Message::Binary(bytes)) => bytes.to_vec(),
            Ok(Message::Text(text)) => text.as_bytes().to_vec(),
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => continue,
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(error) => return Err(Error::Reported(error.to_string())),
        };
        if sink.write_all(&bytes).and_then(|()| sink.flush()).is_err() {
            return Ok(());
        }
    }
}

/// `WebSocketFragmentFD.real_download` with the format's `http_headers`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn download(
    ffmpeg: Option<&Path>,
    headers: Vec<(String, String)>,
    network: &Network<'_>,
    settings: &Settings,
    info: &VideoInfo,
    destination: &Path,
    options: &DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    // `FFmpegFD._call_downloader` without ffmpeg.
    let ffmpeg = ffmpeg.ok_or_else(|| {
        Error::Reported(
            "m3u8 download detected but ffmpeg could not be found. Please install".into(),
        )
    })?;
    let url = info.url.clone();
    let connection: ffmpeg_fd::Connection = Box::new(move |sink| {
        let result = connect(&url, &headers).and_then(|mut socket| forward(&mut socket, sink));
        // Upstream's connection thread prints its exception (a traceback).
        if let Err(error) = result {
            crate::screen::stderr(&format!("websocket connection failed: {error}"));
        }
    });
    ffmpeg_fd::download_piped(
        ffmpeg,
        network,
        settings,
        info,
        destination,
        options,
        connection,
        callback,
    )
}
