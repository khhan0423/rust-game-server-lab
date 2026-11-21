//! Conventions shared by the operator API and its HTTP client.

use super::ApiError;
use axum::http::StatusCode;

/// Operator service credential header.
pub const KEY_HEADER: &str = "x-ops-key";
/// Operator identity header.
pub const NAME_HEADER: &str = "x-operator";
/// Environment variable containing the operator service credential.
pub const KEY_ENV: &str = "PUBLISHER_OPS_API_KEY";

/// Nonempty printable ASCII operator name, at most 128 bytes after trimming.
#[derive(Clone, Debug)]
pub struct OperatorName(String);

impl TryFrom<&str> for OperatorName {
    type Error = ApiError;
    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let text = text.trim();
        if text.is_empty() || text.len() > 128 || !text.bytes().all(|b| (b' '..=b'~').contains(&b))
        {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "operator_required"));
        }
        Ok(Self(text.to_owned()))
    }
}

impl OperatorName {
    /// Returns the validated name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Trimmed explanation for an operator change, at most 1,000 characters.
#[derive(Debug)]
pub struct AuditReason<'a>(&'a str);

impl<'a> TryFrom<&'a str> for AuditReason<'a> {
    type Error = ApiError;
    fn try_from(text: &'a str) -> Result<Self, Self::Error> {
        let text = text.trim();
        if text.is_empty() || text.contains('\0') || text.chars().count() > 1000 {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "reason_required"));
        }
        Ok(Self(text))
    }
}

impl AuditReason<'_> {
    /// Returns the validated reason.
    pub fn as_str(&self) -> &str {
        self.0
    }
}
