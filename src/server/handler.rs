use crate::server::state::AppState;
use axum::extract::State;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::time::Duration;
use tokio::time::Instant;

#[tracing::instrument(level = "debug", skip_all)]
pub async fn metrics(State(state): State<AppState>) -> String {
    if let Ok(mut last_collection) = state.last_collection.try_lock()
        && last_collection.elapsed() > Duration::from_secs(1)
    {
        *last_collection = Instant::now();
        refresh_measurements(&state).await;
    }

    encode_response(&state)
}

#[tracing::instrument(level = "trace", skip_all)]
async fn refresh_measurements(state: &AppState) {
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

#[tracing::instrument(level = "trace", skip_all)]
fn encode_response(state: &AppState) -> String {
    let metric_families = state.registry.gather();
    let encoder = prometheus::TextEncoder::new();

    encoder.encode_to_string(&metric_families).unwrap()
}
