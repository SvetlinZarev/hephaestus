use config::Config;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
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
            enable_stdout: true,
            enable_log_file: true,
            log_file_directory: Some("/tmp/var/log/hephaestus/".to_owned()),
            level: "INFO".to_owned(),
            directives: vec![],
            max_log_files: 7,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Http {
    pub port: u16,
    pub timeout: u64,
}

impl Default for Http {
    fn default() -> Self {
        Self {
            port: 8081,
            timeout: Duration::from_secs(10).as_millis() as u64,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Configuration {
    #[serde(default = "Log::default")]
    pub log: Log,

    #[serde(default = "Http::default")]
    pub http: Http,
}

impl Configuration {
    pub fn load() -> Result<Self, config::ConfigError> {
        let cfg = Config::builder()
            .add_source(
                config::File::with_name("config.toml")
                    .format(config::FileFormat::Toml)
                    .required(false),
            )
            .add_source(
                config::File::with_name("config.yml")
                    .format(config::FileFormat::Toml)
                    .required(false),
            )
            .add_source(
                config::File::with_name("config.json")
                    .format(config::FileFormat::Toml)
                    .required(false),
            )
            .add_source(config::Environment::with_prefix("CFG").separator("__"))
            .build()?;

        cfg.try_deserialize()
    }
}
