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

    #[tracing::instrument(level = "trace", skip_all)]
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

    #[tracing::instrument(level = "trace", skip_all)]
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

    #[tracing::instrument(level = "trace", skip_all)]
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
    #[tracing::instrument(level = "debug", skip_all)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_nut_instance() -> Nut {
        Nut::new(Config::default()).unwrap()
    }

    fn full_params() -> HashMap<String, String> {
        HashMap::from([
            ("battery.runtime".to_string(), "3600".to_string()),
            ("battery.charge".to_string(), "85".to_string()),
            ("ups.load".to_string(), "25".to_string()),
            ("input.voltage".to_string(), "120.5".to_string()),
            ("output.voltage".to_string(), "119.8".to_string()),
            ("ups.power.nominal".to_string(), "1500".to_string()),
            ("ups.realpower.nominal".to_string(), "900".to_string()),
            ("ups.power".to_string(), "375".to_string()),
            ("ups.realpower".to_string(), "225".to_string()),
        ])
    }

    #[test]
    fn test_collect_full_parameters() {
        let nut = create_nut_instance();
        let stats = nut.collect_device_parameters("myups".to_string(), full_params());

        assert_eq!(stats.device_name, "myups");
        assert_eq!(stats.estimated_runtime, Some(3600.0));
        assert!((stats.battery_level.unwrap() - 0.85).abs() < f64::EPSILON);
        assert!((stats.load.unwrap() - 0.25).abs() < f64::EPSILON);
        assert_eq!(stats.input_voltage, Some(120.5));
        assert_eq!(stats.output_voltage, Some(119.8));
        assert_eq!(stats.nominal_apparent_power, Some(1500.0));
        assert_eq!(stats.nominal_real_power, Some(900.0));
        assert_eq!(stats.apparent_power, Some(375.0));
        assert_eq!(stats.real_power, Some(225.0));
    }

    #[test]
    fn test_collect_empty_parameters() {
        let nut = create_nut_instance();
        let stats = nut.collect_device_parameters("myups".to_string(), HashMap::new());

        assert_eq!(stats.device_name, "myups");
        assert!(stats.estimated_runtime.is_none());
        assert!(stats.battery_level.is_none());
        assert!(stats.load.is_none());
        assert!(stats.input_voltage.is_none());
        assert!(stats.output_voltage.is_none());
        assert!(stats.nominal_apparent_power.is_none());
        assert!(stats.nominal_real_power.is_none());
        assert!(stats.apparent_power.is_none());
        assert!(stats.real_power.is_none());
    }

    #[test]
    fn test_collect_derived_real_power() {
        let nut = create_nut_instance();
        let params = HashMap::from([
            ("ups.load".to_string(), "50".to_string()),
            ("ups.realpower.nominal".to_string(), "900".to_string()),
        ]);

        let stats = nut.collect_device_parameters("myups".to_string(), params);
        assert!((stats.real_power.unwrap() - 450.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_collect_derived_apparent_power() {
        let nut = create_nut_instance();
        let params = HashMap::from([
            ("ups.load".to_string(), "40".to_string()),
            ("ups.power.nominal".to_string(), "1500".to_string()),
        ]);

        let stats = nut.collect_device_parameters("myups".to_string(), params);
        assert!((stats.apparent_power.unwrap() - 600.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_collect_no_derived_power_without_load() {
        let nut = create_nut_instance();
        // Has nominal but no load → cannot derive
        let params = HashMap::from([
            ("ups.realpower.nominal".to_string(), "900".to_string()),
            ("ups.power.nominal".to_string(), "1500".to_string()),
        ]);

        let stats = nut.collect_device_parameters("myups".to_string(), params);
        assert!(stats.real_power.is_none());
        assert!(stats.apparent_power.is_none());
    }

    #[test]
    fn test_collect_alternative_keys() {
        let nut = create_nut_instance();
        let params = HashMap::from([
            ("battery.level".to_string(), "90".to_string()),
            ("output.load".to_string(), "30".to_string()),
            ("battery.runtime.low".to_string(), "600".to_string()),
        ]);

        let stats = nut.collect_device_parameters("myups".to_string(), params);
        assert!((stats.battery_level.unwrap() - 0.9).abs() < f64::EPSILON);
        assert!((stats.load.unwrap() - 0.3).abs() < f64::EPSILON);
        assert_eq!(stats.estimated_runtime, Some(600.0));
    }
}
