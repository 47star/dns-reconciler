use tracing::{error, info};

use crate::{
    cloudflare::client::CloudflareClient,
    sync::planner::{Plan, PlanAction},
};

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct ExecutionSummary {
    pub records_created: usize,
    pub records_updated: usize,
    pub records_deleted: usize,
    pub records_unchanged: usize,
    pub records_failed: usize,
}

pub async fn execute_plan(
    cloudflare_client: &CloudflareClient,
    plan: &Plan,
    dry_run: bool,
) -> ExecutionSummary {
    let mut summary = ExecutionSummary::default();

    for action in &plan.actions {
        match action {
            PlanAction::Create { desired } => {
                info!(
                    event = "record_create",
                    name = desired.name,
                    content = %desired.content,
                    ttl = desired.ttl,
                    proxied = desired.proxied,
                    dry_run = dry_run
                );

                if dry_run {
                    summary.records_created += 1;
                    continue;
                }

                match cloudflare_client.create_record(desired).await {
                    Ok(()) => summary.records_created += 1,
                    Err(error) => {
                        summary.records_failed += 1;
                        error!(
                            event = "error",
                            action = "record_create",
                            name = desired.name,
                            error = %error
                        );
                    }
                }
            }
            PlanAction::Update { existing, desired } => {
                info!(
                    event = "record_update",
                    name = desired.name,
                    record_id = existing.id,
                    content = %desired.content,
                    ttl = desired.ttl,
                    proxied = desired.proxied,
                    dry_run = dry_run
                );

                if dry_run {
                    summary.records_updated += 1;
                    continue;
                }

                match cloudflare_client.update_record(&existing.id, desired).await {
                    Ok(()) => summary.records_updated += 1,
                    Err(error) => {
                        summary.records_failed += 1;
                        error!(
                            event = "error",
                            action = "record_update",
                            name = desired.name,
                            record_id = existing.id,
                            error = %error
                        );
                    }
                }
            }
            PlanAction::Delete { existing } => {
                info!(
                    event = "record_delete",
                    name = existing.name,
                    record_id = existing.id,
                    dry_run = dry_run
                );

                if dry_run {
                    summary.records_deleted += 1;
                    continue;
                }

                match cloudflare_client.delete_record(&existing.id).await {
                    Ok(()) => summary.records_deleted += 1,
                    Err(error) => {
                        summary.records_failed += 1;
                        error!(
                            event = "error",
                            action = "record_delete",
                            name = existing.name,
                            record_id = existing.id,
                            error = %error
                        );
                    }
                }
            }
            PlanAction::Noop { existing } => {
                info!(
                    event = "record_skipped",
                    reason = "no_change",
                    name = existing.name,
                    record_id = existing.id
                );
                summary.records_unchanged += 1;
            }
        }
    }

    summary
}
