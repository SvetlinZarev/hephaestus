use hephaestus::config::Configuration;
use hephaestus::logging::setup_logging;
use hephaestus::server::start_server;
use std::sync::Arc;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let configuration = Arc::new(Configuration::load()?);
    let _guard = setup_logging(&configuration.log)?;

    tracing::info!("Starting Hephaestus");
    start_server(configuration).await?;
    tracing::info!("Bye!");

    Ok(())
}
