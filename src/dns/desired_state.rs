use std::{
    collections::{BTreeMap, BTreeSet},
    net::Ipv4Addr,
};

use tracing::warn;

use crate::{
    dns::name::{record_name_from_hostname, Fqdn},
    leases::model::Lease,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DesiredRecord {
    pub name: String,
    pub content: Ipv4Addr,
    pub ttl: u32,
    pub proxied: bool,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct DesiredState {
    pub records: BTreeMap<String, DesiredRecord>,
    pub leases_total: usize,
    pub leases_selected: usize,
}

#[derive(Debug, Clone)]
struct Candidate {
    record: DesiredRecord,
    cltt: u64,
}

pub fn build_desired_state(
    leases: &[Lease],
    subnet_ids: &BTreeSet<u32>,
    suffix: &Fqdn,
    ttl: u32,
    now_epoch_seconds: u64,
) -> DesiredState {
    let mut candidates: BTreeMap<String, Candidate> = BTreeMap::new();

    for lease in leases {
        let subnet_id = match lease.subnet_id {
            Some(subnet_id) if subnet_ids.contains(&subnet_id) => subnet_id,
            Some(subnet_id) => {
                warn!(
                    event = "record_skipped",
                    reason = "subnet_not_managed",
                    subnet_id = subnet_id,
                    ip_address = %lease.ip_address
                );
                continue;
            }
            None => {
                warn!(
                    event = "record_skipped",
                    reason = "missing_subnet_id",
                    ip_address = %lease.ip_address
                );
                continue;
            }
        };

        if !lease.is_active(now_epoch_seconds) {
            warn!(
                event = "record_skipped",
                reason = "inactive_or_expired_lease",
                subnet_id = subnet_id,
                ip_address = %lease.ip_address
            );
            continue;
        }

        let hostname = match lease.hostname.as_deref() {
            Some(hostname) if !hostname.trim().is_empty() => hostname,
            _ => {
                warn!(
                    event = "record_skipped",
                    reason = "missing_hostname",
                    subnet_id = subnet_id,
                    ip_address = %lease.ip_address
                );
                continue;
            }
        };

        let name = match record_name_from_hostname(hostname, suffix) {
            Ok(name) => name,
            Err(error) => {
                warn!(
                    event = "record_skipped",
                    reason = "invalid_hostname",
                    subnet_id = subnet_id,
                    ip_address = %lease.ip_address,
                    hostname = hostname,
                    error = %error
                );
                continue;
            }
        };

        let record = DesiredRecord {
            name: name.clone(),
            content: lease.ip_address,
            ttl,
            proxied: false,
        };
        let candidate = Candidate {
            record,
            cltt: lease.ordering_timestamp(),
        };

        match candidates.get(&name) {
            Some(existing) if prefer_candidate(&candidate, existing) => {
                candidates.insert(name, candidate);
            }
            None => {
                candidates.insert(name, candidate);
            }
            _ => {}
        }
    }

    let records = candidates
        .into_iter()
        .map(|(name, candidate)| (name, candidate.record))
        .collect::<BTreeMap<_, _>>();

    DesiredState {
        leases_total: leases.len(),
        leases_selected: records.len(),
        records,
    }
}

fn prefer_candidate(candidate: &Candidate, existing: &Candidate) -> bool {
    candidate.cltt > existing.cltt
        || (candidate.cltt == existing.cltt
            && u32::from(candidate.record.content) < u32::from(existing.record.content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn lease(hostname: &str, ip: Ipv4Addr, subnet_id: Option<u32>, cltt: u64) -> Lease {
        Lease {
            ip_address: ip,
            hostname: Some(hostname.to_string()),
            state: Some(0),
            subnet_id,
            valid_lft: Some(300),
            cltt: Some(cltt),
            expire: None,
        }
    }

    #[test]
    fn filters_and_normalizes_leases() {
        let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
        let subnets = BTreeSet::from([10]);
        let leases = vec![
            lease("Host01.", Ipv4Addr::new(192, 0, 2, 10), Some(10), 100),
            lease("other", Ipv4Addr::new(192, 0, 2, 11), Some(20), 100),
            lease("_bad", Ipv4Addr::new(192, 0, 2, 12), Some(10), 100),
        ];

        let desired = build_desired_state(&leases, &subnets, &suffix, 300, 150);
        assert_eq!(desired.leases_total, 3);
        assert_eq!(desired.leases_selected, 1);
        assert!(desired.records.contains_key("host01.dhcp.example.com"));
    }

    #[test]
    fn picks_most_recent_hostname_conflict() {
        let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
        let subnets = BTreeSet::from([10]);
        let leases = vec![
            lease("host01", Ipv4Addr::new(192, 0, 2, 10), Some(10), 100),
            lease("host01", Ipv4Addr::new(192, 0, 2, 20), Some(10), 200),
        ];

        let desired = build_desired_state(&leases, &subnets, &suffix, 300, 150);
        let record = desired.records.get("host01.dhcp.example.com").unwrap();
        assert_eq!(record.content, Ipv4Addr::new(192, 0, 2, 20));
    }

    #[test]
    fn tie_breaks_by_lowest_ipv4() {
        let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
        let subnets = BTreeSet::from([10]);
        let leases = vec![
            lease("host01", Ipv4Addr::new(192, 0, 2, 20), Some(10), 100),
            lease("host01", Ipv4Addr::new(192, 0, 2, 10), Some(10), 100),
        ];

        let desired = build_desired_state(&leases, &subnets, &suffix, 300, 150);
        let record = desired.records.get("host01.dhcp.example.com").unwrap();
        assert_eq!(record.content, Ipv4Addr::new(192, 0, 2, 10));
    }
}
