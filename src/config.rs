use std::{collections::BTreeSet, env, fmt, path::PathBuf, time::Duration};

use crate::{
    dns::{name::Fqdn, validation},
    AppError, Result,
};

const DEFAULT_VYOS_DHCP4_LEASES_PATH: &str = "/config/dhcp";
const DEFAULT_CF_API_BASE_URL: &str = "https://api.cloudflare.com/client/v4";
const DEFAULT_CF_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_LEASE_FILE_WATCH_INTERVAL_MILLIS: u64 = 250;
const DEFAULT_LEASE_FILE_DEBOUNCE_MILLIS: u64 = 500;

#[derive(Clone)]
pub struct AppConfig {
    pub vyos_dhcp4_leases_path: PathBuf,
    pub dhcp_subnet_ids: BTreeSet<u32>,
    pub dns_zone: Fqdn,
    pub managed_record_suffix: Fqdn,
    pub cloudflare_zone_id: String,
    pub cloudflare_api_token: String,
    pub default_ttl: u32,
    pub sync_interval: Duration,
    pub log_level: String,
    pub dry_run: bool,
    pub cloudflare_api_base_url: String,
    pub cloudflare_request_timeout: Duration,
    pub lease_file_watch_enabled: bool,
    pub lease_file_watch_interval: Duration,
    pub lease_file_debounce: Duration,
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppConfig")
            .field("vyos_dhcp4_leases_path", &self.vyos_dhcp4_leases_path)
            .field("dhcp_subnet_ids", &self.dhcp_subnet_ids)
            .field("dns_zone", &self.dns_zone)
            .field("managed_record_suffix", &self.managed_record_suffix)
            .field("cloudflare_zone_id", &self.cloudflare_zone_id)
            .field("cloudflare_api_token", &"<redacted>")
            .field("default_ttl", &self.default_ttl)
            .field("sync_interval", &self.sync_interval)
            .field("log_level", &self.log_level)
            .field("dry_run", &self.dry_run)
            .field("cloudflare_api_base_url", &self.cloudflare_api_base_url)
            .field(
                "cloudflare_request_timeout",
                &self.cloudflare_request_timeout,
            )
            .field("lease_file_watch_enabled", &self.lease_file_watch_enabled)
            .field("lease_file_watch_interval", &self.lease_file_watch_interval)
            .field("lease_file_debounce", &self.lease_file_debounce)
            .finish()
    }
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Self::from_getter(|key| env::var(key).ok())
    }

    pub fn from_getter<F>(mut get: F) -> Result<Self>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let vyos_dhcp4_leases_path = PathBuf::from(
            get("VYOS_DHCP4_LEASES_PATH")
                .or_else(|| get("VYOS_DHCP4_LEASES_CSV"))
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_VYOS_DHCP4_LEASES_PATH.to_string()),
        );

        let dhcp_subnet_ids = parse_subnet_ids(&required(&mut get, "DHCP_SUBNET_IDS")?)?;
        let dns_zone = Fqdn::parse(&required(&mut get, "DNS_ZONE")?)?;
        let managed_record_suffix = Fqdn::parse(&required(&mut get, "MANAGED_RECORD_SUFFIX")?)?;
        if !managed_record_suffix.is_strict_subdomain_of(&dns_zone) {
            return Err(AppError::Config(
                "MANAGED_RECORD_SUFFIX must be below DNS_ZONE".to_string(),
            ));
        }

        let cloudflare_zone_id = non_empty(
            required(&mut get, "CLOUDFLARE_ZONE_ID")?,
            "CLOUDFLARE_ZONE_ID",
        )?;
        let cloudflare_api_token = non_empty(
            required(&mut get, "CLOUDFLARE_API_TOKEN")?,
            "CLOUDFLARE_API_TOKEN",
        )?;
        let default_ttl = parse_required_u32(&mut get, "DEFAULT_TTL")?;
        if default_ttl == 0 {
            return Err(AppError::Config(
                "DEFAULT_TTL must be greater than zero".to_string(),
            ));
        }

        let sync_interval_seconds = parse_required_u64(&mut get, "SYNC_INTERVAL_SECONDS")?;
        if sync_interval_seconds == 0 {
            return Err(AppError::Config(
                "SYNC_INTERVAL_SECONDS must be greater than zero".to_string(),
            ));
        }

        let log_level = non_empty(required(&mut get, "LOG_LEVEL")?, "LOG_LEVEL")?;
        let dry_run = parse_optional_bool(get("DRY_RUN").as_deref())?;
        let cloudflare_api_base_url = get("CLOUDFLARE_API_BASE_URL")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_CF_API_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        validate_api_base_url(&cloudflare_api_base_url)?;

        let cloudflare_request_timeout = Duration::from_secs(parse_optional_u64(
            get("CLOUDFLARE_REQUEST_TIMEOUT_SECONDS").as_deref(),
            DEFAULT_CF_TIMEOUT_SECONDS,
            "CLOUDFLARE_REQUEST_TIMEOUT_SECONDS",
        )?);
        let lease_file_watch_enabled =
            parse_optional_bool_with_default(get("LEASE_FILE_WATCH_ENABLED").as_deref(), true)?;
        let lease_file_watch_interval = Duration::from_millis(parse_optional_u64(
            get("LEASE_FILE_WATCH_INTERVAL_MILLIS").as_deref(),
            DEFAULT_LEASE_FILE_WATCH_INTERVAL_MILLIS,
            "LEASE_FILE_WATCH_INTERVAL_MILLIS",
        )?);
        let lease_file_debounce = Duration::from_millis(parse_optional_u64(
            get("LEASE_FILE_DEBOUNCE_MILLIS").as_deref(),
            DEFAULT_LEASE_FILE_DEBOUNCE_MILLIS,
            "LEASE_FILE_DEBOUNCE_MILLIS",
        )?);

        if cloudflare_request_timeout.is_zero() {
            return Err(AppError::Config(
                "CLOUDFLARE_REQUEST_TIMEOUT_SECONDS must be greater than zero".to_string(),
            ));
        }
        if lease_file_watch_interval.is_zero() {
            return Err(AppError::Config(
                "LEASE_FILE_WATCH_INTERVAL_MILLIS must be greater than zero".to_string(),
            ));
        }
        if lease_file_debounce.is_zero() {
            return Err(AppError::Config(
                "LEASE_FILE_DEBOUNCE_MILLIS must be greater than zero".to_string(),
            ));
        }

        Ok(Self {
            vyos_dhcp4_leases_path,
            dhcp_subnet_ids,
            dns_zone,
            managed_record_suffix,
            cloudflare_zone_id,
            cloudflare_api_token,
            default_ttl,
            sync_interval: Duration::from_secs(sync_interval_seconds),
            log_level,
            dry_run,
            cloudflare_api_base_url,
            cloudflare_request_timeout,
            lease_file_watch_enabled,
            lease_file_watch_interval,
            lease_file_debounce,
        })
    }
}

fn required<F>(get: &mut F, key: &str) -> Result<String>
where
    F: FnMut(&str) -> Option<String>,
{
    get(key).ok_or_else(|| AppError::Config(format!("{key} is required")))
}

fn non_empty(value: String, key: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::Config(format!("{key} must not be empty")));
    }
    Ok(trimmed.to_string())
}

fn parse_required_u64<F>(get: &mut F, key: &str) -> Result<u64>
where
    F: FnMut(&str) -> Option<String>,
{
    let value = required(get, key)?;
    value
        .trim()
        .parse::<u64>()
        .map_err(|error| AppError::Config(format!("{key} parse error: {error}")))
}

fn parse_required_u32<F>(get: &mut F, key: &str) -> Result<u32>
where
    F: FnMut(&str) -> Option<String>,
{
    let value = required(get, key)?;
    value
        .trim()
        .parse::<u32>()
        .map_err(|error| AppError::Config(format!("{key} parse error: {error}")))
}

fn parse_optional_u64(value: Option<&str>, default: u64, key: &str) -> Result<u64> {
    match value {
        Some(value) if !value.trim().is_empty() => value
            .trim()
            .parse::<u64>()
            .map_err(|error| AppError::Config(format!("{key} parse error: {error}"))),
        _ => Ok(default),
    }
}

fn parse_optional_bool(value: Option<&str>) -> Result<bool> {
    parse_optional_bool_with_default(value, false)
}

fn parse_optional_bool_with_default(value: Option<&str>, default: bool) -> Result<bool> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(value) => Err(AppError::Config(format!(
            "boolean value must be true or false, got {value}"
        ))),
        None => Ok(default),
    }
}

fn parse_subnet_ids(value: &str) -> Result<BTreeSet<u32>> {
    if value.trim().is_empty() {
        return Err(AppError::Config(
            "DHCP_SUBNET_IDS must not be empty".to_string(),
        ));
    }

    let mut ids = BTreeSet::new();
    for raw in value.split(',') {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(AppError::Config(
                "DHCP_SUBNET_IDS contains an empty item".to_string(),
            ));
        }

        let id = trimmed
            .parse::<u32>()
            .map_err(|error| AppError::Config(format!("DHCP_SUBNET_IDS parse error: {error}")))?;
        if id == 0 {
            return Err(AppError::Config(
                "DHCP_SUBNET_IDS values must be greater than zero".to_string(),
            ));
        }
        ids.insert(id);
    }

    Ok(ids)
}

fn validate_api_base_url(value: &str) -> Result<()> {
    if value.starts_with("https://") || value.starts_with("http://") {
        Ok(())
    } else {
        Err(AppError::Config(
            "CLOUDFLARE_API_BASE_URL must start with http:// or https://".to_string(),
        ))
    }
}

pub fn validate_record_scope(record_name: &str, dns_zone: &Fqdn, suffix: &Fqdn) -> Result<()> {
    let normalized = validation::normalize_fqdn(record_name)?;
    if normalized == dns_zone.as_str() {
        return Err(AppError::DnsName("zone apex is not managed".to_string()));
    }
    if !validation::is_strict_subdomain(&normalized, dns_zone.as_str()) {
        return Err(AppError::DnsName(
            "record name is outside DNS_ZONE".to_string(),
        ));
    }
    if !validation::is_strict_subdomain(&normalized, suffix.as_str()) {
        return Err(AppError::DnsName(
            "record name is outside MANAGED_RECORD_SUFFIX".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn base_env() -> BTreeMap<&'static str, &'static str> {
        BTreeMap::from([
            ("DHCP_SUBNET_IDS", "10,20"),
            ("DNS_ZONE", "example.com."),
            ("MANAGED_RECORD_SUFFIX", "dhcp.example.com."),
            ("CLOUDFLARE_ZONE_ID", "zone-id"),
            ("CLOUDFLARE_API_TOKEN", "secret-value"),
            ("DEFAULT_TTL", "300"),
            ("SYNC_INTERVAL_SECONDS", "300"),
            ("LOG_LEVEL", "info"),
        ])
    }

    #[test]
    fn loads_config_from_getter() {
        let env = base_env();
        let config =
            AppConfig::from_getter(|key| env.get(key).map(|value| value.to_string())).unwrap();
        assert_eq!(config.vyos_dhcp4_leases_path, PathBuf::from("/config/dhcp"));
        assert_eq!(config.dhcp_subnet_ids, BTreeSet::from([10, 20]));
        assert_eq!(config.managed_record_suffix.as_str(), "dhcp.example.com");
        assert!(!format!("{config:?}").contains("secret-value"));
    }

    #[test]
    fn leases_path_overrides_legacy_csv_path() {
        let mut env = base_env();
        env.insert("VYOS_DHCP4_LEASES_PATH", "/config/dhcp");
        env.insert("VYOS_DHCP4_LEASES_CSV", "/config/dhcp/dhcp4-leases.csv");

        let config =
            AppConfig::from_getter(|key| env.get(key).map(|value| value.to_string())).unwrap();

        assert_eq!(config.vyos_dhcp4_leases_path, PathBuf::from("/config/dhcp"));
    }

    #[test]
    fn legacy_csv_path_is_still_supported() {
        let mut env = base_env();
        env.insert("VYOS_DHCP4_LEASES_CSV", "/config/dhcp/dhcp4-leases.csv");

        let config =
            AppConfig::from_getter(|key| env.get(key).map(|value| value.to_string())).unwrap();

        assert_eq!(
            config.vyos_dhcp4_leases_path,
            PathBuf::from("/config/dhcp/dhcp4-leases.csv")
        );
    }

    #[test]
    fn rejects_empty_subnet_ids() {
        let mut env = base_env();
        env.insert("DHCP_SUBNET_IDS", "");
        assert!(AppConfig::from_getter(|key| env.get(key).map(|value| value.to_string())).is_err());
    }

    #[test]
    fn rejects_suffix_outside_zone() {
        let mut env = base_env();
        env.insert("MANAGED_RECORD_SUFFIX", "dhcp.example.internal.");
        assert!(AppConfig::from_getter(|key| env.get(key).map(|value| value.to_string())).is_err());
    }

    #[test]
    fn validates_scope() {
        let zone = Fqdn::parse("example.com.").unwrap();
        let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
        assert!(validate_record_scope("host.dhcp.example.com.", &zone, &suffix).is_ok());
        assert!(validate_record_scope("example.com.", &zone, &suffix).is_err());
        assert!(validate_record_scope("host.example.com.", &zone, &suffix).is_err());
    }
}
