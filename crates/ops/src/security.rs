//! Browser origin, response headers, and per-process CSRF token.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rand::TryRng;
use reqwest::Url;

use crate::AppState;

pub(super) struct Security {
    authority: String,
    origin: String,
    token: String,
}

impl Security {
    pub(super) fn new(addr: SocketAddr) -> anyhow::Result<Self> {
        let mut rng = rand::rngs::SysRng;
        let first = rng.try_next_u64()?;
        let second = rng.try_next_u64()?;
        let authority = addr.to_string();
        let origin = Url::parse(&format!("http://{addr}"))?
            .origin()
            .ascii_serialization();
        Ok(Self {
            authority,
            origin,
            token: format!("{first:016x}{second:016x}"),
        })
    }

    pub(super) fn token(&self) -> &str {
        &self.token
    }

    pub(super) fn accepts(&self, headers: &HeaderMap, token: &str) -> bool {
        check_origin(headers, &self.origin)
            && infra::http::api::constant_time_eq(self.token.as_bytes(), token.as_bytes())
    }
}

fn check_origin(headers: &HeaderMap, expected: &str) -> bool {
    let origins = headers.get_all(header::ORIGIN);
    let referers = headers.get_all(header::REFERER);
    let mut origins = origins.iter();
    let mut referers = referers.iter();
    let origin = origins.next();
    let referer = referers.next();
    (origin.is_some() || referer.is_some())
        && origins.next().is_none()
        && referers.next().is_none()
        && origin.is_none_or(|value| value == expected)
        && referer.is_none_or(|value| {
            value
                .to_str()
                .ok()
                .and_then(|text| Url::parse(text).ok())
                .is_some_and(|url| {
                    url.username().is_empty()
                        && url.password().is_none()
                        && url.origin().ascii_serialization() == expected
                })
        })
}

pub(super) async fn headers(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let accepted = request.headers().get(header::HOST).is_some_and(|host| {
        host == state.security.authority.as_str()
            || state
                .security
                .origin
                .strip_prefix("http://")
                .is_some_and(|authority| host == authority)
    });
    let mut response = if accepted {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "Invalid operations host").into_response()
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; frame-ancestors 'none'; object-src 'none'; form-action 'self'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_same_origin_posts_are_accepted() {
        const LOCAL: &str = "http://127.0.0.1:18300";
        let cases = [
            (Some(LOCAL), None, true),
            (None, Some("http://127.0.0.1:18300/accounts/123"), true),
            (None, None, false),
            (Some("null"), None, false),
            (Some("http://127.0.0.1:18301"), None, false),
            (None, Some("http://127.0.0.1:18300@other.test"), false),
            (Some(LOCAL), Some("https://other.test/"), false),
            (Some("https://other.test"), Some(LOCAL), false),
        ];
        for (origin, referer, expected) in cases {
            let mut headers = HeaderMap::new();
            if let Some(origin) = origin {
                headers.insert(header::ORIGIN, HeaderValue::from_static(origin));
            }
            if let Some(referer) = referer {
                headers.insert(header::REFERER, HeaderValue::from_static(referer));
            }
            assert_eq!(
                check_origin(&headers, LOCAL),
                expected,
                "{origin:?}, {referer:?}"
            );
        }
    }
}
