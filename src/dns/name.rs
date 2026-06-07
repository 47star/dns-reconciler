use std::fmt;

use crate::{dns::validation, Result};

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Fqdn(String);

impl Fqdn {
    pub fn parse(input: &str) -> Result<Self> {
        Ok(Self(validation::normalize_fqdn(input)?))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn absolute(&self) -> String {
        format!("{}.", self.0)
    }

    pub fn is_strict_subdomain_of(&self, parent: &Fqdn) -> bool {
        validation::is_strict_subdomain(self.as_str(), parent.as_str())
    }
}

impl fmt::Display for Fqdn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Debug for Fqdn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Fqdn")
            .field(&self.absolute())
            .finish()
    }
}

pub fn record_name_from_hostname(hostname: &str, suffix: &Fqdn) -> Result<String> {
    let label = validation::normalize_hostname_label(hostname)?;
    let fqdn = Fqdn::parse(&format!("{}.{}", label, suffix.as_str()))?;
    Ok(fqdn.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_record_name() {
        let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
        assert_eq!(
            record_name_from_hostname("Host01.", &suffix).unwrap(),
            "host01.dhcp.example.com"
        );
    }
}
