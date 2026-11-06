use super::*;

/// A black-box comparison, so a wrong token cannot be found one byte at a time.
pub(super) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Where a WebView or a browser socket may carry the token in the URL, because neither
/// can set an `Authorization` header on the request that matters.
pub(super) fn query_token_allowed(path: &str) -> bool {
    path.starts_with("/session/")
        || path.starts_with("/review/")
        || path.starts_with("/ws/")
        || (path.starts_with("/api/sessions/") && path.contains("/annotation/"))
        || (path.starts_with("/api/sessions/") && path.ends_with("/voice"))
}

pub(super) fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then_some(value)
    })
}

/// `%XX` and `+`, because the WebSocket URL is built with `URLEncoder` and the token may
/// legitimately contain a character that has to be escaped.
pub(super) fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

pub(super) fn cookie_value<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    header.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (name, value) = pair.split_once('=')?;
        (name == key).then_some(value)
    })
}

/// The trust boundary: this API can read, edit and delete reviews, and it binds the LAN.
///
/// Loopback is exempt because the CLI and the channel run on the server's own machine.
/// Everything else presents `Authorization: Bearer`, a `?token=` where the WebView needs
/// it, or the `HttpOnly` cookie the page set. No token configured = today's open server.
pub(super) async fn require_token(
    State(state): State<AppState>,
    request: Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(token) = state.token.clone() else {
        return next.run(request).await;
    };
    let path = request.uri().path().to_string();
    // Liveness and the metrics scrape are read-only counters, and a monitor that needs
    // the token to say "alive" is a monitor nobody finishes configuring.
    if path == "/api/health" || path == "/metrics" {
        return next.run(request).await;
    }

    let caller = request
        .extensions()
        .get::<Caller>()
        .and_then(|caller| caller.0)
        .or_else(|| {
            request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|connected| connected.0.ip())
        });
    if caller.is_some_and(|ip| ip.is_loopback()) {
        return next.run(request).await;
    }

    let headers = request.headers();
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let from_query = query_token_allowed(&path)
        .then(|| {
            request
                .uri()
                .query()
                .and_then(|query| query_value(query, "token"))
        })
        .flatten()
        .and_then(percent_decode);
    let from_cookie = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| cookie_value(value, "eink_token"));

    let authorized = [bearer, from_query.as_deref(), from_cookie]
        .into_iter()
        .flatten()
        .any(|candidate| constant_time_eq(candidate.as_bytes(), token.as_bytes()));
    if !authorized {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let mut response = next.run(request).await;
    // The page's images and its JS fetches carry no Authorization header; the cookie is
    // what makes them work without putting the token in the markup.
    if (path.starts_with("/session/") || path.starts_with("/review/"))
        && let Ok(cookie) = header::HeaderValue::from_str(&format!(
            "eink_token={token}; Path=/; HttpOnly; SameSite=Strict"
        ))
    {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

#[cfg(test)]
mod auth_tests {
    use super::{percent_decode, query_token_allowed};

    #[test]
    fn a_query_token_is_decoded_and_only_allowed_where_a_header_cannot_go() {
        assert_eq!(percent_decode("s3%2Bcret").as_deref(), Some("s3+cret"));
        assert_eq!(percent_decode("a+b").as_deref(), Some("a b"));
        assert!(query_token_allowed("/session/abc"));
        assert!(query_token_allowed("/review/abc"));
        assert!(query_token_allowed("/ws/abc"));
        assert!(query_token_allowed("/api/sessions/abc/annotation/0"));
        assert!(!query_token_allowed("/api/sessions"));
    }
}
