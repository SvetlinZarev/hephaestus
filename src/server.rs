use crate::server::shutdown::shutdown_signal;
use crate::server::state::AppState;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use std::error::Error;
use std::net::ToSocketAddrs;
use std::time::Duration;
use tokio::net::TcpListener;
use tower::Layer;
use tower_http::LatencyUnit;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::normalize_path::NormalizePathLayer;
use tower_http::request_id::{
    MakeRequestUuid, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

pub mod handler;
pub mod shutdown;
pub mod state;

pub async fn start_server(state: AppState) -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = state.configuration.clone();
    let router = create_router(state);

    let mut handles = Vec::new();
    for addr in (config.http.address.as_str(), config.http.port).to_socket_addrs()? {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| format!("Could not bind to {}: {}", addr, e))?;

        tracing::info!("Listening on {}", listener.local_addr()?);

        let router = router.clone();
        let handle = tokio::task::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_signal())
                .await
        });

        handles.push(handle);
    }

    if handles.is_empty() {
        return Err(format!(
            "The bind address [{}:{}] did not resolve to any IP addresses",
            config.http.address, config.http.port
        )
        .into());
    }

    for handle in handles {
        match handle.await {
            Ok(Ok(())) => (),
            Ok(Err(e)) => return Err(format!("Server failed: {}", e).into()),
            Err(e) => return Err(format!("Server task panicked: {}", e).into()),
        }
    }

    Ok(())
}

fn create_router(state: AppState) -> Router {
    let router = Router::new()
        .route("/metrics", get(handler::metrics))
        .layer(CatchPanicLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::INTERNAL_SERVER_ERROR,
            Duration::from_millis(state.configuration.http.timeout),
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
        .with_state(state);

    Router::new().fallback_service(NormalizePathLayer::trim_trailing_slash().layer(router))
}
