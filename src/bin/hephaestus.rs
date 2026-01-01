use hephaestus::bootstrap::init_collectors;
use hephaestus::config::{
    Configuration, get_config_base_path, print_config, should_print_config_and_exit,
};
use hephaestus::logging::setup_logging;
use hephaestus::server::start_server;
use hephaestus::server::state::{AppState, Inner};
use std::ops::Sub;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let configuration = Configuration::load(get_config_base_path(std::env::args())?)?;
    if should_print_config_and_exit(std::env::args()) {
        print_config(&configuration)?;
        return Ok(());
    }

    let _guard = setup_logging(&configuration.log)?;
    tracing::info!("Starting Hephaestus");

    let registry = prometheus::Registry::new();
    let collectors = init_collectors(&configuration, &registry)?;

    let state = AppState {
        inner: Arc::new(Inner {
            configuration,
            registry,
            collectors,
            last_collection: Mutex::new(Instant::now().sub(Duration::from_hours(1))),
        }),
    };

    start_server(state).await?;
    tracing::info!("Bye!");

    Ok(())
}
