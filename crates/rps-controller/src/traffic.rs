use crate::db::Database;
use std::{collections::HashMap, time::Duration};
use tokio::sync::mpsc;
use tracing::{debug, warn};

const TRAFFIC_QUEUE_CAPACITY: usize = 65_536;
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const FLUSH_ROUTE_THRESHOLD: usize = 4096;

#[derive(Clone)]
pub struct TrafficAggregator {
    tx: mpsc::Sender<TrafficEvent>,
}

#[derive(Debug)]
pub struct TrafficEvent {
    client_id: String,
    tunnel_id: String,
    rx_bytes: u64,
    tx_bytes: u64,
}

impl TrafficAggregator {
    pub fn channel() -> (Self, mpsc::Receiver<TrafficEvent>) {
        let (tx, rx) = mpsc::channel(TRAFFIC_QUEUE_CAPACITY);
        (Self { tx }, rx)
    }

    pub fn record(&self, client_id: &str, tunnel_id: &str, rx_bytes: u64, tx_bytes: u64) {
        if rx_bytes == 0 && tx_bytes == 0 {
            return;
        }

        let event = TrafficEvent {
            client_id: client_id.to_string(),
            tunnel_id: tunnel_id.to_string(),
            rx_bytes,
            tx_bytes,
        };
        if let Err(err) = self.tx.try_send(event) {
            debug!(error = %err, "traffic aggregation queue full, dropping sample");
        }
    }
}

pub async fn run(db: Database, mut rx: mpsc::Receiver<TrafficEvent>) {
    let mut pending = HashMap::<(String, String), (u64, u64)>::new();
    let mut interval = tokio::time::interval(FLUSH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            Some(event) = rx.recv() => {
                let entry = pending
                    .entry((event.client_id, event.tunnel_id))
                    .or_insert((0, 0));
                entry.0 = entry.0.saturating_add(event.rx_bytes);
                entry.1 = entry.1.saturating_add(event.tx_bytes);

                if pending.len() >= FLUSH_ROUTE_THRESHOLD {
                    flush(&db, &mut pending).await;
                }
            }
            _ = interval.tick() => {
                flush(&db, &mut pending).await;
            }
        }
    }
}

async fn flush(db: &Database, pending: &mut HashMap<(String, String), (u64, u64)>) {
    if pending.is_empty() {
        return;
    }

    let batch = std::mem::take(pending);
    for ((client_id, tunnel_id), (rx_bytes, tx_bytes)) in batch {
        if let Err(err) = db
            .add_traffic(&client_id, &tunnel_id, rx_bytes, tx_bytes)
            .await
        {
            warn!(%client_id, %tunnel_id, error = %err, "failed to flush traffic counters");
        }
    }
}
