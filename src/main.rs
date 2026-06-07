use std::{env, pin::Pin, time::Duration};

use dns_reconciler::{
    cloudflare::client::CloudflareClient,
    config::AppConfig,
    leases::{csv::LeaseCsvClient, watcher::spawn_lease_file_watcher},
    sync::sync_once,
    Result,
};
use tokio::{select, signal, sync::mpsc, time};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    info!(event = "startup");

    let config = AppConfig::from_env()?;
    info!(
        event = "configuration_loaded",
        lease_path = %config.vyos_dhcp4_leases_path.display(),
        subnet_ids = ?config.dhcp_subnet_ids,
        dns_zone = %config.dns_zone,
        managed_record_suffix = %config.managed_record_suffix,
        cloudflare_zone_id = %config.cloudflare_zone_id,
        cloudflare_api_base_url = %config.cloudflare_api_base_url,
        default_ttl = config.default_ttl,
        sync_interval_seconds = config.sync_interval.as_secs(),
        lease_file_watch_enabled = config.lease_file_watch_enabled,
        lease_file_watch_interval_millis = config.lease_file_watch_interval.as_millis() as u64,
        lease_file_debounce_millis = config.lease_file_debounce.as_millis() as u64,
        dry_run = config.dry_run
    );

    let lease_client = LeaseCsvClient::new(config.vyos_dhcp4_leases_path.clone());
    let cloudflare_client = CloudflareClient::new(
        config.cloudflare_api_base_url.clone(),
        config.cloudflare_zone_id.clone(),
        config.cloudflare_api_token.clone(),
        config.cloudflare_request_timeout,
    )?;

    let (lease_event_tx, mut lease_event_rx) = mpsc::channel(32);
    let _watcher = if config.lease_file_watch_enabled {
        Some(spawn_lease_file_watcher(
            lease_client.path().clone(),
            config.lease_file_watch_interval,
            lease_event_tx,
        ))
    } else {
        None
    };

    if let Err(error) = sync_once(&config, &lease_client, &cloudflare_client).await {
        error!(event = "error", component = "sync", error = %error);
    }

    let mut shutdown = Box::pin(shutdown_signal());
    loop {
        select! {
            _ = &mut shutdown => {
                info!(event = "shutdown");
                break;
            }
            event = lease_event_rx.recv() => {
                if event.is_some() {
                    info!(
                        event = "lease_file_event_received",
                        debounce_millis = config.lease_file_debounce.as_millis() as u64
                    );
                    time::sleep(config.lease_file_debounce).await;
                    while lease_event_rx.try_recv().is_ok() {}
                    if let Err(error) = sync_once(&config, &lease_client, &cloudflare_client).await {
                        error!(event = "error", component = "sync", error = %error);
                    }
                }
            }
            _ = sleep_for(config.sync_interval) => {
                if let Err(error) = sync_once(&config, &lease_client, &cloudflare_client).await {
                    error!(event = "error", component = "sync", error = %error);
                }
            }
        }
    }

    Ok(())
}

fn init_tracing() {
    let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let filter = EnvFilter::try_new(log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .flatten_event(true)
        .init();
}

fn sleep_for(duration: Duration) -> Pin<Box<tokio::time::Sleep>> {
    Box::pin(time::sleep(duration))
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let terminate = async {
            match signal(SignalKind::terminate()) {
                Ok(mut stream) => {
                    stream.recv().await;
                }
                Err(_) => std::future::pending::<()>().await,
            }
        };

        select! {
            _ = ctrl_c => {}
            _ = terminate => {}
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await;
    }
}
