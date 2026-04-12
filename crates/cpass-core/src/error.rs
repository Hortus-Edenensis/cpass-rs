use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CpassError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("url parse error: {0}")]
    Url(#[from] url::ParseError),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("session not found in {0}")]
    SessionNotFound(PathBuf),
    #[error("credentials are missing; provide --phone/--password or CPASS_PHONE/CPASS_PASSWORD")]
    MissingCredentials,
    #[error("login failed: {0}")]
    LoginFailed(String),
    #[error("unexpected response: {0}")]
    UnexpectedResponse(String),
    #[error("command is scaffolded but not implemented yet: {0}")]
    UnsupportedCommand(&'static str),
}

pub type Result<T> = std::result::Result<T, CpassError>;
