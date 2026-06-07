use std::{collections::BTreeSet, time::Duration};

use dns_reconciler::{dns::desired_state::build_desired_state, dns::name::Fqdn, leases::csv};

#[test]
fn csv_active_rows_become_desired_records() {
    let csv = br#"address,hwaddr,client_id,valid_lifetime,expire,subnet_id,fqdn_fwd,fqdn_rev,hostname,state,user_context,pool_id
10.100.16.0,fe:ff:ff:a9:59:92,01:fe:ff:ff:a9:59:92,7200,1780811318,10,1,1,,0,,0
10.100.16.0,fe:ff:ff:a9:59:92,01:fe:ff:ff:a9:59:92,7200,1780811318,10,1,1,myhost-10-100-16-0,0,,0
10.100.16.14,00:14:5e:60:33:8a,,7200,1780804384,10,0,0,,2,,0
"#;
    let leases = csv::parse_lease_csv(csv).unwrap();
    let suffix = Fqdn::parse("dhcp.example.com.").unwrap();
    let desired = build_desired_state(&leases, &BTreeSet::from([10]), &suffix, 300, 1_780_800_000);

    assert_eq!(desired.leases_total, 3);
    assert_eq!(desired.leases_selected, 1);
    assert!(desired
        .records
        .contains_key("myhost-10-100-16-0.dhcp.example.com"));
}

#[tokio::test]
async fn csv_client_reads_file() {
    let path = std::env::temp_dir().join(format!(
        "dns-reconciler-leases-{}-{}.csv",
        std::process::id(),
        unique_suffix()
    ));
    tokio::fs::write(
        &path,
        b"address,hwaddr,client_id,valid_lifetime,expire,subnet_id,fqdn_fwd,fqdn_rev,hostname,state,user_context,pool_id\n10.100.16.1,fe:ff:ff:8e:61:8d,01:fe:ff:ff:8e:61:8d,7200,1780811324,10,1,1,myhost-10-100-16-1,0,,0\n",
    )
    .await
    .unwrap();

    let client = csv::LeaseCsvClient::new(path.clone());
    let leases = client.get_ipv4_leases(&BTreeSet::from([10])).await.unwrap();

    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].hostname.as_deref(), Some("myhost-10-100-16-1"));
    tokio::fs::remove_file(path).await.unwrap();
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos()
}
