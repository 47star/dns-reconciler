use serde::{Deserialize, Serialize};

use crate::dns::desired_state::DesiredRecord;

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct CloudflareDnsRecord {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub record_type: String,
    pub content: String,
    pub ttl: u32,
    pub proxied: Option<bool>,
}

impl CloudflareDnsRecord {
    pub fn proxied_value(&self) -> bool {
        self.proxied.unwrap_or(false)
    }
}

#[derive(Debug, Deserialize)]
pub struct CloudflareEnvelope<T> {
    pub success: bool,
    #[serde(default)]
    pub errors: Vec<CloudflareApiMessage>,
    #[serde(default)]
    pub messages: Vec<CloudflareApiMessage>,
    pub result: T,
    pub result_info: Option<CloudflareResultInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CloudflareApiMessage {
    pub code: Option<u64>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct CloudflareResultInfo {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
    pub total_pages: Option<u32>,
    pub count: Option<u32>,
    pub total_count: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct CloudflareDnsRecordPayload<'a> {
    #[serde(rename = "type")]
    pub record_type: &'static str,
    pub name: &'a str,
    pub content: String,
    pub ttl: u32,
    pub proxied: bool,
}

impl<'a> From<&'a DesiredRecord> for CloudflareDnsRecordPayload<'a> {
    fn from(record: &'a DesiredRecord) -> Self {
        Self {
            record_type: "A",
            name: &record.name,
            content: record.content.to_string(),
            ttl: record.ttl,
            proxied: record.proxied,
        }
    }
}

pub fn summarize_messages(messages: &[CloudflareApiMessage]) -> String {
    if messages.is_empty() {
        return "no api error message".to_string();
    }

    messages
        .iter()
        .map(|message| match message.code {
            Some(code) => format!("{code}: {}", message.message),
            None => message.message.clone(),
        })
        .collect::<Vec<_>>()
        .join("; ")
}
