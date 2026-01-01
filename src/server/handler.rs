use crate::server::state::AppState;
use axum::extract::State;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::time::Duration;
use tokio::time::Instant;

pub async fn metrics(State(state): State<AppState>) -> String {
    #[allow(clippy::collapsible_if)]
    if let Ok(mut last_collection) = state.last_collection.try_lock() {
        if last_collection.elapsed() > Duration::from_secs(1) {
            *last_collection = Instant::now();

            let mut futures = FuturesUnordered::new();
            for collector in state.collectors.iter() {
                futures.push(collector.collect());
            }

            while let Some(result) = futures.next().await {
                if let Err(error) = result {
                    tracing::error!(?error, "Metrics collector failed");
                }
            }
        }
    }

    let metric_families = state.registry.gather();
    let encoder = prometheus::TextEncoder::new();

    encoder.encode_to_string(&metric_families).unwrap()
}
