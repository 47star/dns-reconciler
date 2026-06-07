use crate::{AppError, Result};

pub fn normalize_hostname_label(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        return Err(AppError::DnsName("hostname is empty".to_string()));
    }
    if trimmed.contains('.') {
        return Err(AppError::DnsName(
            "hostname must be a single DNS label".to_string(),
        ));
    }

    let label = trimmed.to_ascii_lowercase();
    validate_label(&label)?;
    Ok(label)
}

pub fn normalize_fqdn(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        return Err(AppError::DnsName("fqdn is empty".to_string()));
    }

    let labels: Vec<&str> = trimmed.split('.').collect();
    if labels.iter().any(|label| label.is_empty()) {
        return Err(AppError::DnsName(
            "fqdn contains an empty label".to_string(),
        ));
    }

    let mut normalized = Vec::with_capacity(labels.len());
    for label in labels {
        let label = label.to_ascii_lowercase();
        validate_label(&label)?;
        normalized.push(label);
    }

    let fqdn = normalized.join(".");
    if fqdn.len() + 1 > 253 {
        return Err(AppError::DnsName("fqdn is too long".to_string()));
    }

    Ok(fqdn)
}

pub fn validate_label(label: &str) -> Result<()> {
    if label.is_empty() {
        return Err(AppError::DnsName("label is empty".to_string()));
    }
    if label.len() > 63 {
        return Err(AppError::DnsName("label is too long".to_string()));
    }
    if label == "*" || label.contains('*') {
        return Err(AppError::DnsName(
            "wildcard labels are not allowed".to_string(),
        ));
    }
    if label.starts_with('_') {
        return Err(AppError::DnsName(
            "labels starting with underscore are not allowed".to_string(),
        ));
    }

    let bytes = label.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return Err(AppError::DnsName(
            "label must start and end with an alphanumeric character".to_string(),
        ));
    }

    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
    {
        return Err(AppError::DnsName(
            "label contains an invalid character".to_string(),
        ));
    }

    Ok(())
}

pub fn is_strict_subdomain(name: &str, parent: &str) -> bool {
    name != parent
        && name.len() > parent.len()
        && name.ends_with(parent)
        && name.as_bytes()[name.len() - parent.len() - 1] == b'.'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_hostname_label() {
        assert_eq!(normalize_hostname_label("Host-01.").unwrap(), "host-01");
    }

    #[test]
    fn rejects_hostname_with_multiple_labels() {
        assert!(normalize_hostname_label("host01.example.com.").is_err());
    }

    #[test]
    fn rejects_invalid_labels() {
        assert!(normalize_hostname_label("_host").is_err());
        assert!(normalize_hostname_label("-host").is_err());
        assert!(normalize_hostname_label("host-").is_err());
        assert!(normalize_hostname_label("*").is_err());
        assert!(normalize_hostname_label("bad_name").is_err());
    }

    #[test]
    fn normalizes_fqdn() {
        assert_eq!(
            normalize_fqdn("Dhcp.Example.Com.").unwrap(),
            "dhcp.example.com"
        );
    }

    #[test]
    fn checks_strict_subdomain() {
        assert!(is_strict_subdomain(
            "host.dhcp.example.com",
            "dhcp.example.com"
        ));
        assert!(!is_strict_subdomain("dhcp.example.com", "dhcp.example.com"));
        assert!(!is_strict_subdomain(
            "other.example.com",
            "dhcp.example.com"
        ));
    }
}
