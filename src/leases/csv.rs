use std::{
    collections::{BTreeMap, BTreeSet},
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use serde::Deserialize;
use tokio::{fs, time};
use tracing::{info, warn};

use crate::{leases::model::Lease, AppError, Result};

const DEFAULT_MAX_RETRIES: usize = 3;
const BASE_BACKOFF: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct LeaseCsvClient {
    path: PathBuf,
    max_retries: usize,
}

impl LeaseCsvClient {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }

    #[cfg(test)]
    pub fn with_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries.max(1);
        self
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub async fn get_ipv4_leases(&self, subnet_ids: &BTreeSet<u32>) -> Result<Vec<Lease>> {
        let subnets: Vec<u32> = subnet_ids.iter().copied().collect();
        info!(
            event = "lease_file_read_started",
            path = %self.path.display(),
            subnet_ids = ?subnets
        );

        let mut last_error = None;
        for attempt in 1..=self.max_retries {
            match self.read_once().await {
                Ok(leases) => {
                    info!(
                        event = "lease_file_read_completed",
                        leases_total = leases.len(),
                        attempt = attempt
                    );
                    return Ok(leases);
                }
                Err(error) => {
                    warn!(
                        event = "error",
                        component = "lease_file",
                        attempt = attempt,
                        error = %error
                    );
                    last_error = Some(error);
                }
            }

            if attempt < self.max_retries {
                time::sleep(backoff_delay(attempt)).await;
            }
        }

        Err(last_error.unwrap_or(AppError::LeaseSource(
            "lease file read ended without a result".to_string(),
        )))
    }

    async fn read_once(&self) -> Result<Vec<Lease>> {
        let files = discover_lease_files(&self.path).await?;
        if files.is_empty() {
            return Err(AppError::LeaseSource(format!(
                "no lease files found under {}",
                self.path.display()
            )));
        }

        let files_total = files.len();
        let mut rows_by_address = BTreeMap::new();
        for file in files {
            let bytes = fs::read(&file.path).await?;
            for lease in parse_lease_csv(&bytes)? {
                rows_by_address.insert(lease.ip_address, lease);
            }
        }

        info!(
            event = "lease_file_set_loaded",
            path = %self.path.display(),
            files_total = files_total,
            leases_total = rows_by_address.len()
        );

        Ok(rows_by_address.into_values().collect())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LeaseFile {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub len: u64,
}

pub async fn discover_lease_files(path: &Path) -> Result<Vec<LeaseFile>> {
    let metadata = fs::metadata(path).await?;

    if metadata.is_dir() {
        discover_from_directory(path, "dhcp4-leases.csv").await
    } else {
        let directory = path.parent().ok_or_else(|| {
            AppError::LeaseSource(format!("lease file has no parent: {}", path.display()))
        })?;
        let prefix = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                AppError::LeaseSource(format!("lease file name is invalid: {}", path.display()))
            })?;
        discover_from_directory(directory, prefix).await
    }
}

async fn discover_from_directory(directory: &Path, prefix: &str) -> Result<Vec<LeaseFile>> {
    let mut entries = fs::read_dir(directory).await?;
    let mut files = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(prefix) {
            continue;
        }

        let metadata = entry.metadata().await?;
        if !metadata.is_file() {
            continue;
        }

        files.push(LeaseFile {
            path,
            modified: metadata.modified().ok(),
            len: metadata.len(),
        });
    }

    files.sort_by(|left, right| {
        left.modified
            .cmp(&right.modified)
            .then_with(|| left.path.cmp(&right.path))
    });

    Ok(files)
}

#[derive(Debug, Deserialize)]
struct LeaseCsvRow {
    address: Ipv4Addr,
    valid_lifetime: u64,
    expire: u64,
    subnet_id: u32,
    hostname: String,
    state: i64,
}

pub fn parse_lease_csv(bytes: &[u8]) -> Result<Vec<Lease>> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(bytes);
    let mut leases = Vec::new();

    for row in reader.deserialize::<LeaseCsvRow>() {
        let row = row?;
        leases.push(Lease {
            ip_address: row.address,
            hostname: normalize_optional_string(row.hostname),
            state: Some(row.state),
            subnet_id: Some(row.subnet_id),
            valid_lft: Some(row.valid_lifetime),
            cltt: None,
            expire: Some(row.expire),
        });
    }

    Ok(leases)
}

fn normalize_optional_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn backoff_delay(attempt: usize) -> Duration {
    let multiplier = 2_u32.saturating_pow((attempt - 1).min(4) as u32);
    BASE_BACKOFF * multiplier
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vyos_lease_csv() {
        let csv = br#"address,hwaddr,client_id,valid_lifetime,expire,subnet_id,fqdn_fwd,fqdn_rev,hostname,state,user_context,pool_id
10.100.16.0,fe:ff:ff:a9:59:92,01:fe:ff:ff:a9:59:92,7200,1780811318,10,1,1,,0,,0
10.100.16.0,fe:ff:ff:a9:59:92,01:fe:ff:ff:a9:59:92,7200,1780811318,10,1,1,myhost-10-100-16-0,0,,0
10.100.16.14,00:14:5e:60:33:8a,,7200,1780804384,10,0,0,,2,,0
"#;

        let leases = parse_lease_csv(csv).unwrap();
        assert_eq!(leases.len(), 3);
        assert_eq!(leases[0].hostname, None);
        assert_eq!(leases[1].hostname.as_deref(), Some("myhost-10-100-16-0"));
        assert_eq!(leases[1].subnet_id, Some(10));
        assert_eq!(leases[2].state, Some(2));
    }

    #[tokio::test]
    async fn merges_files_by_modified_time_and_row_order() {
        let directory = std::env::temp_dir().join(format!(
            "dns-reconciler-lease-set-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir(&directory).await.unwrap();

        let older = directory.join("dhcp4-leases.csv.2");
        let newer = directory.join("dhcp4-leases.csv");
        fs::write(
            &older,
            b"address,hwaddr,client_id,valid_lifetime,expire,subnet_id,fqdn_fwd,fqdn_rev,hostname,state,user_context,pool_id\n10.100.16.4,fe:ff:ff:1f:b0:8e,,7200,1780901923,10,0,0,mongja,0,,0\n10.100.16.3,fe:ff:ff:85:28:d8,,7200,1780901765,10,0,0,securedbserver,0,,0\n",
        )
        .await
        .unwrap();
        time::sleep(Duration::from_millis(20)).await;
        fs::write(
            &newer,
            b"address,hwaddr,client_id,valid_lifetime,expire,subnet_id,fqdn_fwd,fqdn_rev,hostname,state,user_context,pool_id\n10.100.16.4,fe:ff:ff:1f:b0:8e,,0,1780894723,10,0,0,mongja,3,,0\n10.100.16.4,fe:ff:ff:1f:b0:8e,,0,1780894723,10,0,0,,2,,0\n10.100.16.5,fe:ff:ff:b8:41:25,,7200,1780902777,10,0,0,sdb,0,,0\n",
        )
        .await
        .unwrap();

        let client = LeaseCsvClient::new(directory.clone()).with_retries(1);
        let leases = client.get_ipv4_leases(&BTreeSet::from([10])).await.unwrap();

        assert_eq!(leases.len(), 3);
        let lease_4 = leases
            .iter()
            .find(|lease| lease.ip_address == Ipv4Addr::new(10, 100, 16, 4))
            .unwrap();
        assert_eq!(lease_4.state, Some(2));
        assert_eq!(lease_4.hostname, None);
        assert!(leases
            .iter()
            .any(|lease| lease.hostname.as_deref() == Some("securedbserver")));
        assert!(leases
            .iter()
            .any(|lease| lease.hostname.as_deref() == Some("sdb")));

        fs::remove_dir_all(directory).await.unwrap();
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos()
    }
}
