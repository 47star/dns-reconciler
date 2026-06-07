use std::collections::{BTreeMap, VecDeque};

use crate::{
    cloudflare::model::CloudflareDnsRecord,
    dns::{
        desired_state::DesiredRecord,
        name::Fqdn,
        validation::{is_strict_subdomain, normalize_fqdn},
    },
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PlanAction {
    Create {
        desired: DesiredRecord,
    },
    Update {
        existing: CloudflareDnsRecord,
        desired: DesiredRecord,
    },
    Delete {
        existing: CloudflareDnsRecord,
    },
    Noop {
        existing: CloudflareDnsRecord,
    },
}

impl PlanAction {
    pub fn name(&self) -> &str {
        match self {
            Self::Create { desired } => &desired.name,
            Self::Update { desired, .. } => &desired.name,
            Self::Delete { existing } | Self::Noop { existing } => &existing.name,
        }
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct PlanSummary {
    pub creates: usize,
    pub updates: usize,
    pub deletes: usize,
    pub unchanged: usize,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct Plan {
    pub actions: Vec<PlanAction>,
    pub summary: PlanSummary,
}

pub fn build_plan(
    desired: &BTreeMap<String, DesiredRecord>,
    current: &[CloudflareDnsRecord],
    suffix: &Fqdn,
) -> Plan {
    let mut current_by_name: BTreeMap<String, Vec<CloudflareDnsRecord>> = BTreeMap::new();

    for record in current {
        if !record.record_type.eq_ignore_ascii_case("A") {
            continue;
        }

        let Ok(name) = normalize_fqdn(&record.name) else {
            continue;
        };

        if !is_strict_subdomain(&name, suffix.as_str()) {
            continue;
        }

        current_by_name
            .entry(name)
            .or_default()
            .push(record.clone());
    }

    for records in current_by_name.values_mut() {
        records.sort_by(|left, right| left.id.cmp(&right.id));
    }

    let mut actions = Vec::new();
    for (name, desired_record) in desired {
        match current_by_name.remove(name) {
            None => actions.push(PlanAction::Create {
                desired: desired_record.clone(),
            }),
            Some(records) => plan_existing_records(&mut actions, desired_record, records),
        }
    }

    for records in current_by_name.into_values() {
        for existing in records {
            actions.push(PlanAction::Delete { existing });
        }
    }

    let summary = summarize(&actions);
    Plan { actions, summary }
}

fn plan_existing_records(
    actions: &mut Vec<PlanAction>,
    desired_record: &DesiredRecord,
    records: Vec<CloudflareDnsRecord>,
) {
    let mut records = VecDeque::from(records);

    if let Some(index) = records
        .iter()
        .position(|record| record_matches(record, desired_record))
    {
        let primary = records.remove(index).expect("record index exists");
        actions.push(PlanAction::Noop { existing: primary });
    } else {
        let primary = records
            .pop_front()
            .expect("existing record collection is not empty");
        actions.push(PlanAction::Update {
            existing: primary,
            desired: desired_record.clone(),
        });
    }

    for duplicate in records {
        actions.push(PlanAction::Delete {
            existing: duplicate,
        });
    }
}

fn record_matches(existing: &CloudflareDnsRecord, desired: &DesiredRecord) -> bool {
    normalize_fqdn(&existing.name)
        .map(|name| name == desired.name)
        .unwrap_or(false)
        && existing.record_type.eq_ignore_ascii_case("A")
        && existing.content == desired.content.to_string()
        && existing.ttl == desired.ttl
        && existing.proxied_value() == desired.proxied
}

fn summarize(actions: &[PlanAction]) -> PlanSummary {
    let mut summary = PlanSummary::default();
    for action in actions {
        match action {
            PlanAction::Create { .. } => summary.creates += 1,
            PlanAction::Update { .. } => summary.updates += 1,
            PlanAction::Delete { .. } => summary.deletes += 1,
            PlanAction::Noop { .. } => summary.unchanged += 1,
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, net::Ipv4Addr};

    fn desired(name: &str, ip: Ipv4Addr) -> DesiredRecord {
        DesiredRecord {
            name: name.to_string(),
            content: ip,
            ttl: 300,
            proxied: false,
        }
    }

    fn current(id: &str, name: &str, content: &str, ttl: u32) -> CloudflareDnsRecord {
        CloudflareDnsRecord {
            id: id.to_string(),
            name: name.to_string(),
            record_type: "A".to_string(),
            content: content.to_string(),
            ttl,
            proxied: Some(false),
        }
    }

    #[test]
    fn creates_missing_record() {
        let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
        let desired_records = BTreeMap::from([(
            "host01.dhcp.example.com".to_string(),
            desired("host01.dhcp.example.com", Ipv4Addr::new(192, 0, 2, 10)),
        )]);

        let plan = build_plan(&desired_records, &[], &suffix);
        assert_eq!(plan.summary.creates, 1);
    }

    #[test]
    fn updates_changed_record() {
        let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
        let desired_records = BTreeMap::from([(
            "host01.dhcp.example.com".to_string(),
            desired("host01.dhcp.example.com", Ipv4Addr::new(192, 0, 2, 10)),
        )]);
        let current_records = vec![current(
            "id-1",
            "host01.dhcp.example.com",
            "192.0.2.11",
            300,
        )];

        let plan = build_plan(&desired_records, &current_records, &suffix);
        assert_eq!(plan.summary.updates, 1);
    }

    #[test]
    fn leaves_unmanaged_record() {
        let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
        let current_records = vec![current("id-1", "host01.example.com", "192.0.2.11", 300)];

        let plan = build_plan(&BTreeMap::new(), &current_records, &suffix);
        assert!(plan.actions.is_empty());
    }

    #[test]
    fn removes_extra_managed_records() {
        let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
        let current_records = vec![current("id-1", "old.dhcp.example.com", "192.0.2.11", 300)];

        let plan = build_plan(&BTreeMap::new(), &current_records, &suffix);
        assert_eq!(plan.summary.deletes, 1);
    }

    #[test]
    fn handles_duplicate_current_records() {
        let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
        let desired_records = BTreeMap::from([(
            "host01.dhcp.example.com".to_string(),
            desired("host01.dhcp.example.com", Ipv4Addr::new(192, 0, 2, 10)),
        )]);
        let current_records = vec![
            current("id-1", "host01.dhcp.example.com", "192.0.2.11", 300),
            current("id-2", "host01.dhcp.example.com", "192.0.2.10", 300),
        ];

        let plan = build_plan(&desired_records, &current_records, &suffix);
        assert_eq!(plan.summary.unchanged, 1);
        assert_eq!(plan.summary.deletes, 1);
    }
}
