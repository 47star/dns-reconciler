use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use tokio::{sync::mpsc, time};
use tracing::{info, warn};

use crate::leases::csv::discover_lease_files;

#[derive(Debug, Clone, Eq, PartialEq)]
struct LeaseFileSignature {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
}

pub fn spawn_lease_file_watcher(
    path: PathBuf,
    interval: Duration,
    sender: mpsc::Sender<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!(
            event = "lease_file_watch_started",
            path = %path.display(),
            interval_millis = interval.as_millis() as u64
        );

        let mut previous = read_signature(&path).await;
        loop {
            time::sleep(interval).await;
            let current = read_signature(&path).await;

            match (&previous, &current) {
                (Ok(previous), Ok(current)) if previous != current => {
                    info!(
                        event = "lease_file_changed",
                        path = %path.display()
                    );
                    if sender.try_send(()).is_err() {
                        warn!(
                            event = "error",
                            component = "lease_file_watcher",
                            message = "event channel is full"
                        );
                    }
                }
                (Err(_), Ok(_)) => {
                    info!(
                        event = "lease_file_changed",
                        path = %path.display()
                    );
                    if sender.try_send(()).is_err() {
                        warn!(
                            event = "error",
                            component = "lease_file_watcher",
                            message = "event channel is full"
                        );
                    }
                }
                (Ok(_), Err(error)) | (Err(error), Err(_)) => {
                    warn!(
                        event = "error",
                        component = "lease_file_watcher",
                        path = %path.display(),
                        error = %error
                    );
                }
                _ => {}
            }

            previous = current;
        }
    })
}

async fn read_signature(path: &Path) -> std::io::Result<Vec<LeaseFileSignature>> {
    let files = discover_lease_files(path)
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?;

    Ok(files
        .into_iter()
        .map(|file| LeaseFileSignature {
            path: file.path,
            modified: file.modified,
            len: file.len,
        })
        .collect())
}
