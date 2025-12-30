use crate::metrics::ups::{DataSource, UpsDeviceStats, UpsStats};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub address: String,
    pub port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            address: "localhost".to_owned(),
            port: 3493,
        }
    }
}

pub struct Nut {
    config: Config,
}

impl Nut {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    async fn list_ups_devices(
        &self,
        reader: &mut BufReader<ReadHalf<'_>>,
        writer: &mut WriteHalf<'_>,
    ) -> anyhow::Result<Vec<String>> {
        writer
            .write_all(b"LIST UPS\n")
            .await
            .context("Failed to send LIST UPS command")?;

        let mut names = Vec::new();
        let mut line = String::new();

        while reader.read_line(&mut line).await? > 0 {
            let trimmed = line.trim();
            if trimmed == "END LIST UPS" {
                break;
            }

            // Format: UPS <name> "Description"
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.get(0) == Some(&"UPS") {
                if let Some(&name) = parts.get(1) {
                    names.push(name.to_owned());
                }
            }

            line.clear();
        }

        tracing::debug!(?names, "Discovered UPS devices");
        Ok(names)
    }

    async fn list_device_parameters(
        &self,
        reader: &mut BufReader<ReadHalf<'_>>,
        writer: &mut WriteHalf<'_>,
        ups_name: &str,
    ) -> anyhow::Result<HashMap<String, String>> {
        let cmd = format!("LIST VAR {}\n", ups_name);
        writer
            .write_all(cmd.as_bytes())
            .await
            .context("Failed to sent LIST VAR command")?;

        let mut params = HashMap::new();
        let mut line = String::new();

        while reader.read_line(&mut line).await? > 0 {
            let trimmed = line.trim();
            if trimmed.starts_with("END LIST VAR") {
                break;
            }

            // Format: VAR <upsname> <parameter.name> "<value>"
            let parts: Vec<&str> = trimmed.splitn(4, ' ').collect();
            if parts.len() >= 4 {
                let key = parts[2].to_string();
                let value = parts[3].trim_matches('"').to_string();
                params.insert(key, value);
            }

            line.clear();
        }

        tracing::debug!(?params, ?ups_name, "Discovered UPS parameters");
        Ok(params)
    }

    fn collect_device_parameters(
        &self,
        name: String,
        params: HashMap<String, String>,
    ) -> UpsDeviceStats {
        let parse = |key: &str| params.get(key).and_then(|v| v.parse::<f64>().ok());

        UpsDeviceStats {
            device_name: name,
            estimated_runtime: parse("battery.runtime").unwrap_or(0.0),
            battery_level: parse("battery.charge").unwrap_or(0.0) / 100.0,
            input_voltage: parse("input.voltage"),
            output_voltage: parse("output.voltage"),
            load: parse("ups.load").unwrap_or(0.0) / 100.0,
            real_power: parse("ups.realpower").unwrap_or(0.0),

            // Fallback logic: some UPS use ups.realpower, others ups.power
            apparent_power: parse("ups.power")
                .or_else(|| parse("ups.realpower"))
                .unwrap_or(0.0),
        }
    }
}

impl DataSource for Nut {
    fn ups_stats(&self) -> impl Future<Output = anyhow::Result<UpsStats>> + Send {
        let addr = format!("{}:{}", self.config.address, self.config.port);
        tracing::debug!(%addr, "Trying to connect to NUT server");

        async move {
            let mut stream = TcpStream::connect(&addr)
                .await
                .with_context(|| format!("Failed to connect to NUT server at [{}]", addr))?;

            let (reader, mut writer) = stream.split();
            let mut reader = BufReader::new(reader);

            let mut devices = vec![];
            let ups_devices = self.list_ups_devices(&mut reader, &mut writer).await?;

            for device in ups_devices {
                let parameters = self
                    .list_device_parameters(&mut reader, &mut writer, &device)
                    .await?;

                let device_stats = self.collect_device_parameters(device, parameters);
                devices.push(device_stats);
            }

            Ok(UpsStats {
                timestamp: Instant::now(),
                devices,
            })
        }
    }
}
