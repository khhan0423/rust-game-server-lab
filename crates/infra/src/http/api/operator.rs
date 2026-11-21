//! Conventions shared by the operator API and its HTTP client.

/// Operator service credential header.
pub const KEY_HEADER: &str = "x-ops-key";
/// Operator identity header.
pub const NAME_HEADER: &str = "x-operator";
/// Environment variable containing the operator service credential.
pub const KEY_ENV: &str = "PUBLISHER_OPS_API_KEY";
