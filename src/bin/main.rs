use hephaestus::config::Configuration;
use hephaestus::logging::setup_logging;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let configuration = Configuration::load()?;
    let _guard = setup_logging(&configuration.log)?;

    tracing::info!("Starting Hephaestus");


    tracing::info!("Bye!");
    Ok(())
}
