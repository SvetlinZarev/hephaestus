use crate::server::state::AppState;
use axum::extract::State;
use futures::stream::FuturesUnordered;
use futures::StreamExt;

pub async fn health_check() -> &'static str {
    "OK"
}

pub async fn metrics(State(state): State<AppState>) -> String {
    let mut futures = FuturesUnordered::new();

    for collector in state.collectors.iter() {
        futures.push(collector.collect());
    }

    while let Some(result) = futures.next().await {
        if let Err(error) = result {
            tracing::error!(?error, "a metrics collector failed");
        }
    }

    let metric_families = state.registry.gather();
    let encoder = prometheus::TextEncoder::new();

    encoder.encode_to_string(&metric_families).unwrap()
}
