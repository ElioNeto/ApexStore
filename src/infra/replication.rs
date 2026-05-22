use crate::core::log_record::LogRecord;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// The role of this node in replication topology.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReplicationRole {
    Primary,
    Replica,
}

impl Default for ReplicationRole {
    fn default() -> Self {
        Self::Primary
    }
}

impl std::fmt::Display for ReplicationRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Primary => write!(f, "primary"),
            Self::Replica => write!(f, "replica"),
        }
    }
}

/// Configuration for primary-replica replication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationConfig {
    pub role: ReplicationRole,
    #[serde(default)]
    pub replica_endpoints: Vec<String>,
    #[serde(default = "default_sync_interval")]
    pub sync_interval_ms: u64,
}

fn default_sync_interval() -> u64 {
    100
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            role: ReplicationRole::Primary,
            replica_endpoints: Vec::new(),
            sync_interval_ms: default_sync_interval(),
        }
    }
}

/// A batch of WAL records shipped from primary to replica over HTTP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationFrame {
    pub records: Vec<LogRecord>,
    pub sequence: u64,
}

/// Statistics about replication activity.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReplicationStats {
    pub frames_sent: u64,
    pub frames_received: u64,
    pub records_sent: u64,
    pub records_received: u64,
    pub errors: u64,
    pub last_error: Option<String>,
    pub connected: bool,
}

/// Throttling/backoff state for a single replica endpoint.
struct ReplicaState {
    endpoint: String,
    consecutive_failures: u64,
}

/// Replication client running on the Primary node.
///
/// Accumulates WAL records and periodically ships them in batches to all
/// configured replica endpoints via HTTP POST.  Uses exponential backoff
/// when a replica is unreachable.
pub struct ReplicationClient {
    config: ReplicationConfig,
    record_tx: mpsc::UnboundedSender<Vec<LogRecord>>,
    stats: Arc<parking_lot::Mutex<ReplicationStats>>,
}

impl ReplicationClient {
    /// Start the replication background task and return a client handle.
    ///
    /// The returned `JoinHandle` runs the shipping loop; it can be aborted
    /// during shutdown by calling `.abort()` on it.
    pub fn start(config: ReplicationConfig) -> (Self, tokio::task::JoinHandle<()>) {
        let stats = Arc::new(parking_lot::Mutex::new(ReplicationStats::default()));
        let (record_tx, mut record_rx) = mpsc::unbounded_channel::<Vec<LogRecord>>();

        let client = Self {
            config: config.clone(),
            record_tx,
            stats: stats.clone(),
        };

        let endpoints: Vec<ReplicaState> = config
            .replica_endpoints
            .iter()
            .map(|ep| ReplicaState {
                endpoint: ep.clone(),
                consecutive_failures: 0,
            })
            .collect();

        let sync_interval = Duration::from_millis(config.sync_interval_ms);
        let stats_clone = stats.clone();

        let handle = tokio::spawn(async move {
            let mut batch: Vec<LogRecord> = Vec::new();
            let mut sequence: u64 = 0;
            let mut flush_timer = tokio::time::interval(sync_interval);
            let client =
                reqwest::Client::builder()
                    .timeout(Duration::from_secs(30))
                    .build();

            let http_client = match client {
                Ok(c) => c,
                Err(e) => {
                    let mut s = stats_clone.lock();
                    s.errors += 1;
                    s.last_error = Some(format!("failed to build HTTP client: {}", e));
                    return;
                }
            };

            let mut replicas = endpoints;

            loop {
                tokio::select! {
                    Some(records) = record_rx.recv() => {
                        batch.extend(records);
                    }
                    _ = flush_timer.tick() => {
                        if batch.is_empty() {
                            continue;
                        }

                        let current_batch = std::mem::take(&mut batch);
                        sequence += 1;

                        let frame = ReplicationFrame {
                            records: current_batch,
                            sequence,
                        };

                        let payload = match serde_json::to_vec(&frame) {
                            Ok(p) => p,
                            Err(e) => {
                                let mut s = stats_clone.lock();
                                s.errors += 1;
                                s.last_error = Some(format!("serialization error: {}", e));
                                continue;
                            }
                        };

                        for replica in &mut replicas {
                            let url = format!(
                                "{}/admin/replicate",
                                replica.endpoint.trim_end_matches('/')
                            );

                            // Exponential backoff: 100ms, 200ms, 400ms, ... up to ~51s
                            if replica.consecutive_failures > 0 {
                                let backoff_ms = 100u64
                                    .saturating_mul(1u64 << replica.consecutive_failures.min(9));
                                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                            }

                            match http_client
                                .post(&url)
                                .header("Content-Type", "application/json")
                                .body(payload.clone())
                                .send()
                                .await
                            {
                                Ok(resp) => {
                                    if resp.status().is_success() {
                                        let mut s = stats_clone.lock();
                                        s.frames_sent += 1;
                                        s.records_sent += frame.records.len() as u64;
                                        s.connected = true;
                                        replica.consecutive_failures = 0;
                                    } else {
                                        let mut s = stats_clone.lock();
                                        s.errors += 1;
                                        s.last_error = Some(format!(
                                            "replica {} returned {}",
                                            replica.endpoint,
                                            resp.status()
                                        ));
                                        s.connected = false;
                                        replica.consecutive_failures =
                                            replica.consecutive_failures.saturating_add(1);
                                    }
                                }
                                Err(e) => {
                                    let mut s = stats_clone.lock();
                                    s.errors += 1;
                                    s.last_error = Some(format!(
                                        "failed to send to {}: {}",
                                        replica.endpoint, e
                                    ));
                                    s.connected = false;
                                    replica.consecutive_failures =
                                        replica.consecutive_failures.saturating_add(1);
                                }
                            }
                        }
                    }
                }
            }
        });

        (client, handle)
    }

    /// Submit records for replication (called after WAL writes on the primary).
    ///
    /// This is non-blocking; records are buffered in an unbounded channel and
    /// shipped in batches by the background task.
    pub fn ship_records(&self, records: Vec<LogRecord>) {
        let _ = self.record_tx.send(records);
    }

    /// Return the current replication statistics.
    pub fn stats(&self) -> ReplicationStats {
        self.stats.lock().clone()
    }

    /// Return a reference to the config.
    pub fn config(&self) -> &ReplicationConfig {
        &self.config
    }
}
