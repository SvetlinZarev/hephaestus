use crate::config::Configuration;
use crate::server::shutdown::shutdown_signal;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tower::Layer;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::normalize_path::NormalizePathLayer;
use tower_http::request_id::{
    MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tower_http::LatencyUnit;
use tracing::Level;

pub mod handler;
pub mod shutdown;

pub async fn start_server(
    configuration: Arc<Configuration>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let router = create_router(configuration.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], configuration.http.port));
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("Could not bind to {}: {}", addr, e))?;

    tracing::info!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn create_router(cfg: Arc<Configuration>) -> Router {
    let router = Router::new()
        .route("/metrics", get(|| handler::metrics()))
        .route("/health", get(|| handler::health_check()))
        .layer(CatchPanicLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::INTERNAL_SERVER_ERROR,
            Duration::from_millis(cfg.http.timeout),
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<Body>| {
                    let request_id = request
                        .extensions()
                        .get::<RequestId>()
                        .map(|id| id.header_value().to_str().unwrap_or("unknown"))
                        .unwrap_or("unknown");

                    tracing::info_span!(
                        "http_request",
                        request_id = %request_id,
                        method = %request.method(),
                        uri = %request.uri(),
                    )
                })
                .on_response(
                    DefaultOnResponse::new()
                        .level(Level::INFO)
                        .latency_unit(LatencyUnit::Millis),
                ),
        )
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .with_state(cfg);

    Router::new().fallback_service(NormalizePathLayer::trim_trailing_slash().layer(router))
}
