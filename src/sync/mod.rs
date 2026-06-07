pub mod executor;
pub mod planner;

use std::time::{SystemTime, UNIX_EPOCH};

use tracing::{error, info};

use crate::{
    cloudflare::client::CloudflareClient,
    config::AppConfig,
    dns::desired_state::build_desired_state,
    leases::csv::LeaseCsvClient,
    sync::{executor::execute_plan, planner::build_plan},
    Result,
};

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct SyncSummary {
    pub leases_total: usize,
    pub leases_selected: usize,
    pub records_created: usize,
    pub records_updated: usize,
    pub records_deleted: usize,
    pub records_unchanged: usize,
    pub records_failed: usize,
}

pub async fn sync_once(
    config: &AppConfig,
    lease_client: &LeaseCsvClient,
    cloudflare_client: &CloudflareClient,
) -> Result<SyncSummary> {
    info!(event = "sync_started");

    let leases = lease_client
        .get_ipv4_leases(&config.dhcp_subnet_ids)
        .await
        .inspect_err(|error| {
            error!(
                event = "error",
                component = "lease_file",
                message = "lease file read failed; cloudflare changes skipped",
                error = %error
            );
        })?;

    let now = current_epoch_seconds();
    let desired = build_desired_state(
        &leases,
        &config.dhcp_subnet_ids,
        &config.managed_record_suffix,
        config.default_ttl,
        now,
    );

    let current = cloudflare_client
        .list_a_records()
        .await
        .inspect_err(|error| {
            error!(
                event = "error",
                component = "cloudflare",
                message = "record list failed; changes skipped",
                error = %error
            );
        })?;

    let plan = build_plan(&desired.records, &current, &config.managed_record_suffix);
    let execution = execute_plan(cloudflare_client, &plan, config.dry_run).await;

    let summary = SyncSummary {
        leases_total: desired.leases_total,
        leases_selected: desired.leases_selected,
        records_created: execution.records_created,
        records_updated: execution.records_updated,
        records_deleted: execution.records_deleted,
        records_unchanged: execution.records_unchanged,
        records_failed: execution.records_failed,
    };

    info!(
        event = "sync_completed",
        leases_total = summary.leases_total,
        leases_selected = summary.leases_selected,
        records_created = summary.records_created,
        records_updated = summary.records_updated,
        records_deleted = summary.records_deleted,
        records_unchanged = summary.records_unchanged,
        records_failed = summary.records_failed,
        dry_run = config.dry_run
    );

    Ok(summary)
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
