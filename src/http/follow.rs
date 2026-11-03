//! Requests as upstream's urllib handler sends them. The client follows no
//! redirects itself (`Policy::none`); [`send`] follows them here, like
//! yt-dlp's `RedirectHandler` over urllib's `HTTPRedirectHandler`, so each
//! hop's headers can be written in http.client's order: `Host` first. hyper
//! appends `Host` after every other header, and some Cloudflare rules refuse
//! that (Dailymotion's manifests answer 403, issue #74).
use crate::{Error, Result};
use reqwest::{
    Method, StatusCode,
    blocking::{Client, Request, RequestBuilder, Response},
    header::{
        AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, COOKIE, HOST, HeaderMap, HeaderValue,
        LOCATION, PROXY_AUTHORIZATION, TRANSFER_ENCODING, WWW_AUTHENTICATE,
    },
};
use std::collections::HashMap;

/// urllib's `HTTPRedirectHandler.max_redirections` (distinct URLs) and
/// `max_repeats` (visits to one URL).
const MAX_REDIRECTIONS: usize = 10;
const MAX_REPEATS: usize = 4;

/// Send `request`, following redirects.
pub(super) fn send(request: RequestBuilder) -> Result<Response> {
    let (client, request) = request.build_split();
    follow(&client, request?)
}

fn follow(client: &Client, mut request: Request) -> Result<Response> {
    let mut visited: HashMap<url::Url, usize> = HashMap::new();
    loop {
        // A body that cannot be sent twice ends the chain at its redirect,
        // as in reqwest.
        let retry = request.try_clone();
        let response = client.execute(host_first(request))?;
        let status = response.status();
        if !matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        let Some(target) = response
            .headers()
            .get(LOCATION)
            .and_then(|location| location.to_str().ok())
            .and_then(|location| response.url().join(&location.replace(' ', "%20")).ok())
            .filter(|target| matches!(target.scheme(), "http" | "https"))
        else {
            return Ok(response);
        };
        let Some(mut next) = retry else {
            return Ok(response);
        };
        // `http_error_302`'s loop check.
        if visited.get(&target).copied().unwrap_or(0) >= MAX_REPEATS
            || visited.len() >= MAX_REDIRECTIONS
        {
            return Err(Error::RedirectLoop {
                status: status.as_u16(),
                reason: status.canonical_reason().unwrap_or("").to_owned(),
            });
        }
        *visited.entry(target.clone()).or_default() += 1;
        redirect(&mut next, status, &target);
        request = next;
    }
}

/// `RedirectHandler.redirect_request`: the next hop of `request`.
fn redirect(request: &mut Request, status: StatusCode, target: &url::Url) {
    let headers = request.headers_mut();
    // Set explicitly or not, the jar adds its own cookies for the target.
    headers.remove(COOKIE);
    // Credentials an extractor set for this request stay with its origin
    // (reqwest's `remove_sensitive_headers`); global `--add-headers`, which
    // the client adds to every request, go to every host, as upstream's do.
    if request.url().origin() != target.origin() {
        let headers = request.headers_mut();
        for name in [AUTHORIZATION, PROXY_AUTHORIZATION, WWW_AUTHENTICATE] {
            headers.remove(name);
        }
    }
    // `get_redirect_method`; a changed method drops the payload.
    let method = request.method().clone();
    let changed = if (status == StatusCode::SEE_OTHER && method != Method::HEAD)
        || (matches!(status.as_u16(), 301 | 302) && method == Method::POST)
    {
        Method::GET
    } else {
        method.clone()
    };
    if changed != method {
        *request.method_mut() = changed;
        *request.body_mut() = None;
        let headers = request.headers_mut();
        for name in [CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING] {
            headers.remove(name);
        }
    }
    *request.url_mut() = target.clone();
}

/// `request` with `Host` as its first header (http.client's `putrequest`),
/// unless a caller set one.
fn host_first(mut request: Request) -> Request {
    let url = request.url();
    let Some(host) = url.host_str() else {
        return request;
    };
    let host = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let Ok(host) = HeaderValue::from_str(&host) else {
        return request;
    };
    let old = std::mem::take(request.headers_mut());
    let mut headers = HeaderMap::with_capacity(old.len() + 1);
    match old.get(HOST) {
        Some(value) => headers.insert(HOST, value.clone()),
        None => headers.insert(HOST, host),
    };
    let mut name = None;
    for (key, value) in old {
        // `HeaderMap::into_iter` names only a name's first value.
        if let Some(key) = key {
            name = Some(key);
        }
        let key = name.clone().expect("the first entry has a name");
        if key != HOST {
            headers.append(key, value);
        }
    }
    *request.headers_mut() = headers;
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_comes_first_with_a_non_default_port() {
        let client = Client::new();
        let request = client
            .get("http://example.test:8080/path")
            .header("X-Custom", "1")
            .header("Accept", "a")
            .header("Accept", "b")
            .build()
            .unwrap();
        let request = host_first(request);
        let names: Vec<_> = request.headers().keys().map(|n| n.as_str()).collect();
        assert_eq!(names, ["host", "x-custom", "accept"]);
        assert_eq!(request.headers()[HOST], "example.test:8080");
        let accept: Vec<_> = request.headers().get_all("accept").iter().collect();
        assert_eq!(accept, ["a", "b"]);
    }

    #[test]
    fn a_caller_host_is_kept_and_moved_first() {
        let request = Client::new()
            .get("https://example.test/")
            .header("X-Custom", "1")
            .header(HOST, "other.test")
            .build()
            .unwrap();
        let request = host_first(request);
        let names: Vec<_> = request.headers().keys().map(|n| n.as_str()).collect();
        assert_eq!(names, ["host", "x-custom"]);
        assert_eq!(request.headers()[HOST], "other.test");
    }

    #[test]
    fn redirects_change_the_method_like_upstream() {
        let client = Client::new();
        let target = url::Url::parse("https://other.test/next").unwrap();
        for (method, status, expected) in [
            (Method::POST, 301, Method::GET),
            (Method::POST, 302, Method::GET),
            (Method::POST, 303, Method::GET),
            (Method::PUT, 303, Method::GET),
            (Method::HEAD, 303, Method::HEAD),
            (Method::POST, 307, Method::POST),
            (Method::POST, 308, Method::POST),
            (Method::PUT, 301, Method::PUT),
        ] {
            let mut request = client
                .request(method.clone(), "https://example.test/")
                .header(COOKIE, "a=1")
                .header(AUTHORIZATION, "secret")
                .header(CONTENT_TYPE, "text/plain")
                .header("X-Custom", "1")
                .body("data")
                .build()
                .unwrap();
            redirect(&mut request, StatusCode::from_u16(status).unwrap(), &target);
            assert_eq!(request.method(), expected, "{method} {status}");
            assert_eq!(request.url(), &target);
            let headers = request.headers();
            assert!(!headers.contains_key(COOKIE));
            assert!(!headers.contains_key(AUTHORIZATION));
            assert!(headers.contains_key("x-custom"));
            let kept = expected == method;
            assert_eq!(request.body().is_some(), kept, "{method} {status}");
            assert_eq!(
                headers.contains_key(CONTENT_TYPE),
                kept,
                "{method} {status}"
            );
        }
    }
}
