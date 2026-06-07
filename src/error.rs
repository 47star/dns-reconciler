use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("dns name error: {0}")]
    DnsName(String),

    #[error("lease source error: {0}")]
    LeaseSource(String),

    #[error("cloudflare api error: status={status:?} message={message}")]
    CloudflareApi {
        status: Option<u16>,
        message: String,
    },

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),

    #[error("request timeout: {0}")]
    Timeout(&'static str),
}

pub type Result<T> = std::result::Result<T, AppError>;
