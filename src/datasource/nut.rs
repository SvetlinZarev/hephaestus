use crate::metrics::ups::{DataSource, UpsDeviceStats, UpsStats};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub address: String,
    pub port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_owned(),
            port: 3493,
        }
    }
}

pub struct Nut {
    addr: SocketAddr,
}

impl Nut {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let addr = format!("{}:{}", config.address, config.port);

        let addr: SocketAddr = addr
            .parse()
            .with_context(|| format!("Invalid socket address: [{}]", addr))?;

        Ok(Self { addr })
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
            if parts.len() >= 2 && parts[0] == "UPS" {
                names.push(parts[1].to_owned());
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
        device_name: String,
        params: HashMap<String, String>,
    ) -> UpsDeviceStats {
        let as_percents = |x: f64| -> f64 { x / 100.0 };
        let find = |keys: &[&str]| {
            keys.iter()
                .find_map(|&key| params.get(key).and_then(|v| v.parse::<f64>().ok()))
        };

        let estimated_runtime = find(&["battery.runtime", "battery.runtime.low"]);

        let battery_level =
            find(&["battery.charge", "battery.level", "battery.charge.approx"]).map(as_percents);

        let load = find(&["ups.load", "output.load"]).map(as_percents);
        let input_voltage = find(&["input.voltage"]);
        let output_voltage = find(&["output.voltage"]);

        let nominal_apparent_power = find(&["ups.power.nominal", "output.power.nominal"]);
        let nominal_real_power = find(&["ups.realpower.nominal", "output.realpower.nominal"]);

        let real_power = find(&["ups.realpower", "output.realpower"]).or({
            match (nominal_real_power, load) {
                (Some(nom_w), Some(load)) if nom_w > 0.0 => Some(nom_w * load),
                _ => None,
            }
        });

        let apparent_power =
            find(&["ups.power", "output.power"]).or(match (nominal_apparent_power, load) {
                (Some(nom_va), Some(load)) if nom_va > 0.0 => Some(nom_va * load),
                _ => None,
            });

        UpsDeviceStats {
            device_name,
            estimated_runtime,
            battery_level,
            input_voltage,
            output_voltage,
            load,
            real_power,
            apparent_power,
            nominal_apparent_power,
            nominal_real_power,
        }
    }
}

impl DataSource for Nut {
    async fn ups_stats(&self) -> anyhow::Result<UpsStats> {
        let mut stream = TcpStream::connect(&self.addr)
            .await
            .with_context(|| format!("Failed to connect to NUT server at [{}]", &self.addr))?;

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
