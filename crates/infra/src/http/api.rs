//! JSON API extraction, rejection responses and key authentication.

pub mod operator;

use crate::config::{ConfigError, Secret};
use axum::{
    Json,
    extract::{FromRequest, Request},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

/// Public API failure with a stable machine-readable error code.
#[derive(Debug, thiserror::Error)]
#[error("{code}")]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
}

/// Stable JSON error envelope returned by an API, independent of its code vocabulary.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct ErrorBody<T = String> {
    error: T,
}

impl<T> ErrorBody<T> {
    /// Returns the code supplied by the API.
    pub fn code(&self) -> &T {
        &self.error
    }

    /// Extracts the code from the response body.
    pub fn into_code(self) -> T {
        self.error
    }
}

impl ApiError {
    /// Reports a public code without exposing internal details.
    pub const fn new(status: StatusCode, code: &'static str) -> Self {
        Self { status, code }
    }

    /// Rejection for malformed request bodies, paths, or query fields.
    pub const INVALID: Self = Self::new(StatusCode::BAD_REQUEST, "invalid_request");

    /// Reports an unmatched route on a JSON API.
    pub fn not_found() -> std::future::Ready<Self> {
        std::future::ready(Self::new(StatusCode::NOT_FOUND, "not_found"))
    }

    /// Reports an unsupported method on a JSON API route.
    pub fn method_not_allowed() -> std::future::Ready<Self> {
        std::future::ready(Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
        ))
    }

    /// Records an internal failure without returning implementation details to callers.
    pub fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(%error, "API operation failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorBody { error: self.code })).into_response()
    }
}

/// A typed JSON object with a uniform rejection response.
#[derive(Debug)]
pub struct ApiJson<T>(T);

impl<T> ApiJson<T> {
    /// Returns the validated request body.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(object) = Json::<Map<String, Value>>::from_request(req, state)
            .await
            .map_err(|_| ApiError::INVALID)?;
        serde_json::from_value(Value::Object(object))
            .map(Self)
            .map_err(|_| ApiError::INVALID)
    }
}

/// One sensitive header shared by a caller and its corresponding API.
#[derive(Debug)]
pub struct ApiKey {
    name: HeaderName,
    value: HeaderValue,
}

impl ApiKey {
    /// Loads a nonempty HTTP header value and marks it sensitive.
    pub fn from_env(name: &'static str, env: &str) -> Result<Self, ConfigError> {
        let key = Secret::from_env(env)?;
        Self::parse(name, key.expose()).ok_or_else(|| ConfigError::InvalidHeader {
            name: env.to_owned(),
        })
    }

    pub(crate) fn parse(name: &'static str, value: &str) -> Option<Self> {
        let mut value = HeaderValue::from_str(value)
            .ok()
            .filter(|v| !v.is_empty())?;
        value.set_sensitive(true);
        Some(Self {
            name: HeaderName::from_static(name),
            value,
        })
    }

    /// Adds this credential to an outgoing request's headers.
    pub fn insert(&self, headers: &mut HeaderMap) {
        headers.insert(self.name.clone(), self.value.clone());
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        self.value.as_bytes()
    }

    /// Accepts exactly one matching credential.
    pub fn verify(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        let mut values = headers.get_all(&self.name).iter();
        if values
            .next()
            .is_some_and(|value| constant_time_eq(self.value.as_bytes(), value.as_bytes()))
            && values.next().is_none()
        {
            Ok(())
        } else {
            Err(ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))
        }
    }
}

/// Compares every supplied byte without early exit on different contents.
pub fn constant_time_eq(expected: &[u8], supplied: &[u8]) -> bool {
    let mut diff = expected.len() ^ supplied.len();
    for (index, byte) in supplied.iter().enumerate() {
        let other = expected.get(index).copied().unwrap_or_default();
        diff |= usize::from(std::hint::black_box(byte ^ other));
    }
    diff == 0
}
