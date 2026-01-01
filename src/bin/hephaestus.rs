use hephaestus::bootstrap::init_collectors;
use hephaestus::config::Configuration;
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
    let configuration = Configuration::load(get_config_base_path())?;
    let _guard = setup_logging(&configuration.log)?;
    if should_print_config_and_exit() {
        print_config(&configuration)?;
        return Ok(());
    }

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

fn get_config_base_path() -> String {
    let mut base_path = "./";

    let args = std::env::args().collect::<Vec<_>>();
    if args.len() >= 2 {
        base_path = &args[1];
    }

    base_path.to_owned()
}

fn should_print_config_and_exit() -> bool {
    std::env::args()
        .inspect(|arg| tracing::debug!(argument=%arg))
        .any(|arg| arg == "--print-config")
}

fn print_config(config: &Configuration) -> anyhow::Result<()> {
    println!("{}", toml::to_string(config)?);
    Ok(())
}
