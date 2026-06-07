use std::time::Duration;

use reqwest::{header::RETRY_AFTER, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::time;
use tracing::warn;

use crate::{
    cloudflare::model::{
        summarize_messages, CloudflareDnsRecord, CloudflareDnsRecordPayload, CloudflareEnvelope,
    },
    dns::desired_state::DesiredRecord,
    AppError, Result,
};

const DEFAULT_MAX_RETRIES: usize = 3;
const BASE_BACKOFF: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct CloudflareClient {
    http: reqwest::Client,
    api_base_url: String,
    zone_id: String,
    api_token: String,
    max_retries: usize,
}

impl CloudflareClient {
    pub fn new(
        api_base_url: String,
        zone_id: String,
        api_token: String,
        timeout: Duration,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("dns-reconciler/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Self {
            http,
            api_base_url: api_base_url.trim_end_matches('/').to_string(),
            zone_id,
            api_token,
            max_retries: DEFAULT_MAX_RETRIES,
        })
    }

    #[cfg(test)]
    pub fn with_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries.max(1);
        self
    }

    pub async fn list_a_records(&self) -> Result<Vec<CloudflareDnsRecord>> {
        let mut page = 1_u32;
        let mut records = Vec::new();

        loop {
            let url = self.records_url();
            let page_string = page.to_string();
            let envelope: CloudflareEnvelope<Vec<CloudflareDnsRecord>> = self
                .request_json(|| {
                    self.authorize(self.http.get(&url)).query(&[
                        ("type", "A"),
                        ("page", page_string.as_str()),
                        ("per_page", "100"),
                    ])
                })
                .await?;

            ensure_success(&envelope)?;
            records.extend(envelope.result);

            let total_pages = envelope
                .result_info
                .as_ref()
                .and_then(|info| info.total_pages)
                .unwrap_or(1);
            if page >= total_pages {
                break;
            }
            page += 1;
        }

        Ok(records)
    }

    pub async fn create_record(&self, desired: &DesiredRecord) -> Result<()> {
        let url = self.records_url();
        let payload = CloudflareDnsRecordPayload::from(desired);
        let envelope: CloudflareEnvelope<Value> = self
            .request_json(|| self.authorize(self.http.post(&url)).json(&payload))
            .await?;
        ensure_success(&envelope)
    }

    pub async fn update_record(&self, record_id: &str, desired: &DesiredRecord) -> Result<()> {
        let url = format!("{}/{}", self.records_url(), record_id);
        let payload = CloudflareDnsRecordPayload::from(desired);
        let envelope: CloudflareEnvelope<Value> = self
            .request_json(|| self.authorize(self.http.put(&url)).json(&payload))
            .await?;
        ensure_success(&envelope)
    }

    pub async fn delete_record(&self, record_id: &str) -> Result<()> {
        let url = format!("{}/{}", self.records_url(), record_id);
        let envelope: CloudflareEnvelope<Value> = self
            .request_json(|| self.authorize(self.http.delete(&url)))
            .await?;
        ensure_success(&envelope)
    }

    fn records_url(&self) -> String {
        format!("{}/zones/{}/dns_records", self.api_base_url, self.zone_id)
    }

    fn authorize(&self, request: RequestBuilder) -> RequestBuilder {
        request.bearer_auth(&self.api_token)
    }

    async fn request_json<T, F>(&self, make_request: F) -> Result<T>
    where
        T: DeserializeOwned,
        F: Fn() -> RequestBuilder,
    {
        let mut last_error = None;

        for attempt in 1..=self.max_retries {
            match make_request().send().await {
                Ok(response) => {
                    let status = response.status();
                    if is_retryable(status) && attempt < self.max_retries {
                        let delay =
                            retry_after_delay(&response).unwrap_or_else(|| backoff_delay(attempt));
                        warn!(
                            event = "error",
                            component = "cloudflare",
                            attempt = attempt,
                            status = status.as_u16(),
                            retry_after_ms = delay.as_millis() as u64
                        );
                        time::sleep(delay).await;
                        continue;
                    }

                    let bytes = response.bytes().await?;
                    if !status.is_success() {
                        return Err(AppError::CloudflareApi {
                            status: Some(status.as_u16()),
                            message: summarize_http_body(&bytes),
                        });
                    }

                    return Ok(serde_json::from_slice(&bytes)?);
                }
                Err(error) if attempt < self.max_retries && is_retryable_error(&error) => {
                    warn!(
                        event = "error",
                        component = "cloudflare",
                        attempt = attempt,
                        error = %error
                    );
                    last_error = Some(error);
                    time::sleep(backoff_delay(attempt)).await;
                }
                Err(error) => return Err(AppError::Http(error)),
            }
        }

        Err(last_error
            .map(AppError::Http)
            .unwrap_or_else(|| AppError::CloudflareApi {
                status: None,
                message: "cloudflare request ended without a result".to_string(),
            }))
    }
}

fn ensure_success<T>(envelope: &CloudflareEnvelope<T>) -> Result<()> {
    let _ = &envelope.messages;
    if envelope.success {
        Ok(())
    } else {
        Err(AppError::CloudflareApi {
            status: None,
            message: summarize_messages(&envelope.errors),
        })
    }
}

fn is_retryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn is_retryable_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

fn backoff_delay(attempt: usize) -> Duration {
    let multiplier = 2_u32.saturating_pow((attempt - 1).min(4) as u32);
    BASE_BACKOFF * multiplier
}

fn retry_after_delay(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

fn summarize_http_body(bytes: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        if let Some(errors) = value.get("errors").and_then(Value::as_array) {
            let messages = errors
                .iter()
                .filter_map(|error| error.get("message").and_then(Value::as_str))
                .collect::<Vec<_>>();
            if !messages.is_empty() {
                return messages.join("; ");
            }
        }
        return value.to_string();
    }

    String::from_utf8_lossy(bytes).chars().take(512).collect()
}
