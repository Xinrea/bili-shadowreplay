pub mod user_agent_generator;

use crate::errors::RecorderError;
use reqwest::header::HeaderValue;

/// Build a header value from user-provided data without panicking.
///
/// Cookie strings, referers and user agents come from accounts or config, so an
/// invalid byte must surface as a typed error instead of a panic.
pub fn header_value(name: &str, value: &str) -> Result<HeaderValue, RecorderError> {
    HeaderValue::from_str(value).map_err(|_| RecorderError::InvalidHeaderValue {
        name: name.to_string(),
    })
}
