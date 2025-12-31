use crate::datasource::nut;
use crate::metrics::{
    cpu_frequency, cpu_usage, disk_io, disk_smart, memory_usage, network_io, ups,
};
use config::Config;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Configuration {
    pub log: Log,
    pub http: Http,
    pub collector: Collectors,
    pub datasource: DataSources,
}

impl Configuration {
    pub fn load(config_path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let config_path = config_path.as_ref();
        let defaults = Configuration::default();
        let defaults = serde_json::to_string(&defaults)?;

        let cfg = Config::builder()
            .add_source(config::File::from_str(&defaults, config::FileFormat::Json))
            .add_source(
                config::File::with_name(config_path.join("config.toml").to_string_lossy().as_ref())
                    .format(config::FileFormat::Toml)
                    .required(false),
            )
            .add_source(
                config::File::with_name(config_path.to_string_lossy().as_ref()).required(false),
            )
            .add_source(config::Environment::with_prefix("CFG").separator("__"))
            .build()?;

        Ok(cfg.try_deserialize()?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Log {
    pub enable_stdout: bool,
    pub enable_log_file: bool,
    pub log_file_directory: Option<String>,
    pub level: String,
    pub directives: Vec<String>,
    pub max_log_files: usize,
}

impl Default for Log {
    fn default() -> Self {
        Self {
            enable_stdout: false,
            enable_log_file: true,
            log_file_directory: Some("/var/log/hephaestus/".to_owned()),
            level: "INFO".to_owned(),
            directives: vec![],
            max_log_files: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Http {
    pub port: u16,
    pub address: String,
    pub timeout: u64,
}

impl Default for Http {
    fn default() -> Self {
        Self {
            port: 9123,
            address: "0.0.0.0".to_owned(),
            timeout: Duration::from_secs(10).as_millis() as u64,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Collectors {
    pub cpu_usage: cpu_usage::Config,
    pub cpu_frequency: cpu_frequency::Config,
    pub memory_usage: memory_usage::Config,
    pub network_io: network_io::Config,
    pub disk_io: disk_io::Config,
    pub disk_temp: disk_smart::Config,
    pub ups: ups::Config,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DataSources {
    pub nut: nut::Config,
}
