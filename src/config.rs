use crate::datasource::nut;
use crate::metrics::{
    cpu_frequency, cpu_usage, disk_io, disk_smart, docker, memory_usage, network_io, ups, zfs_arc,
    zfs_dataset,
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
    pub zfs_arc: zfs_arc::Config,
    pub zfs_dataset: zfs_dataset::Config,
    pub docker: docker::Config,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DataSources {
    pub nut: nut::Config,
}

pub fn get_config_base_path<I, S>(args: I) -> anyhow::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    const ARG_CONFIG: &str = "--config";
    const ARG_CONFIG_EQ: &str = "--config=";
    const CURRENT_DIRECTORY: &str = "./";

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let arg = arg.as_ref();

        if arg == ARG_CONFIG {
            let raw = iter.next();
            let path = raw
                .as_ref()
                .map(|v| v.as_ref().trim())
                .filter(|v| !v.is_empty())
                .filter(|v| !v.starts_with("-"))
                .ok_or_else(|| {
                    anyhow::anyhow!("Expected configuration path after {}", ARG_CONFIG)
                })?;

            return Ok(path.to_owned());
        }

        if let Some(path) = arg.strip_prefix(ARG_CONFIG_EQ) {
            let path = path.trim();

            if path.is_empty() {
                return Err(anyhow::anyhow!(
                    "Expected configuration path for parameter {}",
                    ARG_CONFIG_EQ
                ));
            }

            return Ok(path.to_owned());
        }
    }

    Ok(CURRENT_DIRECTORY.to_owned())
}

pub fn should_print_config_and_exit<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .inspect(|arg| tracing::debug!(argument = %arg.as_ref()))
        .any(|arg| arg.as_ref() == "--print-config")
}

pub fn print_config(config: &Configuration) -> anyhow::Result<()> {
    println!("{}", toml::to_string(config)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::config::{get_config_base_path, should_print_config_and_exit};

    #[test]
    fn test_should_print_config_and_exit_cases() {
        let cases = [
            (vec![], false),
            (vec!["--hello"], false),
            (vec!["--print-config"], true),
            (vec!["/path", "--print-config", "--hello"], true),
        ];

        for (args, expected) in cases {
            assert_eq!(
                should_print_config_and_exit(&args),
                expected,
                "args={:?}; expected={}",
                args,
                expected
            );
        }
    }

    #[test]
    fn test_get_config_base_path() {
        // (input args, expected result, is_error)
        let cases = [
            (vec!["app"], "./", false),
            (vec!["app", "--config", "/tmp/config"], "/tmp/config", false),
            (vec!["app", "--foo", "--config", "path"], "path", false),
            (vec!["app", "--config", "--print-config"], "", true),
            (vec!["app", "--config=/etc/config"], "/etc/config", false),
            (vec!["app", "--config=another", "--bar"], "another", false),
            (vec!["app", "--config"], "", true),
            (vec!["app", "--config="], "", true),
            (vec!["app", "--config=   "], "", true),
        ];

        for (ref args, expected, is_error) in cases {
            let result = get_config_base_path(args);

            if is_error {
                assert!(result.is_err(), "Expected error for args: {:?}", args);
            } else {
                assert_eq!(result.unwrap(), expected, "Failed for args: {:?}", args);
            }
        }
    }
}
