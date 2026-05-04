//! Plugin error type.

use thiserror::Error;

/// Errors raised by the Gofile plugin.
#[derive(Debug, Error)]
pub enum PluginError {
    #[error("JSON error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    #[error("Gofile HTTP returned status {status}: {message}")]
    HttpStatus { status: u16, message: String },

    #[error("host function response invalid: {0}")]
    HostResponse(String),

    #[error("URL is not a recognised Gofile resource: {0}")]
    UnsupportedUrl(String),

    #[error("Gofile content is offline or removed: {0}")]
    Offline(String),

    #[error("Gofile API rejected the request: {0}")]
    ApiError(String),

    #[error("Gofile folder is empty (no children)")]
    EmptyFolder,

    #[error("Gofile file id {0} not found in folder")]
    FileNotInFolder(String),
}
