use crate::metrics::disk_smart::{DataSource, Device, NvmeDevice, SataDevice, SmartReports};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde_json::Value;
use std::future::Future;
use tokio::process::Command;
use tokio::time::Instant;

enum DeviceReport {
    Sata(SataDevice),
    Nvme(NvmeDevice),
}

pub struct SmartCtl {
    //
}

impl SmartCtl {
    pub fn new() -> Self {
        Self {}
    }

    async fn scan_devices(&self) -> anyhow::Result<Vec<String>> {
        let output = Command::new("smartctl")
            .args(["--scan", "--json"])
            .output()
            .await?;

        let json: Value = serde_json::from_slice(&output.stdout)?;
        let mut paths = Vec::new();

        if let Some(devices) = json["devices"].as_array() {
            for dev in devices {
                if let Some(name) = dev["name"].as_str() {
                    paths.push(name.to_string());
                }
            }
        }

        Ok(paths)
    }

    async fn query_device(&self, path: &str) -> anyhow::Result<Option<DeviceReport>> {
        let output = Command::new("smartctl")
            .args(["-a", "--json", "--nocheck", "standby", path])
            .output()
            .await?;

        // Check exit code 2 (skipped due to standby/sleep)
        if !output.status.success() && output.status.code() == Some(2) {
            return Ok(None);
        }

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "smartctl failed with status: {:?}",
                output.status
            ));
        }

        let json: Value = serde_json::from_slice(&output.stdout)?;
        let info = Device {
            device: path.to_string(),
            model: json["model_name"].as_str().unwrap_or("Unknown").to_string(),
            serial_number: json["serial_number"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string(),
        };

        let dev_type = json["device"]["type"].as_str().unwrap_or("");

        let report = if dev_type == "nvme" {
            DeviceReport::Nvme(self.parse_nvme(info, &json))
        } else {
            DeviceReport::Sata(self.parse_sata(info, &json))
        };

        Ok(Some(report))
    }

    fn parse_nvme(&self, info: Device, json: &Value) -> NvmeDevice {
        let health = &json["nvme_smart_health_information_log"];

        NvmeDevice {
            device: info,
            temperature: health["temperature"].as_f64(),
            available_spare: health["available_spare"].as_f64().map(|x| x / 100.0),
            percent_used: health["percentage_used"].as_f64().map(|x| x / 100.0),
            data_units_read: health["data_units_read"].as_u64(),
            data_units_written: health["data_units_written"].as_u64(),
            host_reads: health["host_reads"].as_u64(),
            host_writes: health["host_writes"].as_u64(),
            power_on_hours: health["power_on_hours"].as_u64(),
            unsafe_shutdowns: health["unsafe_shutdowns"].as_u64(),
            media_errors: health["media_errors"].as_u64(),
        }
    }

    fn parse_sata(&self, info: Device, json: &Value) -> SataDevice {
        let mut device = SataDevice::new(info);

        if let Some(table) = json["ata_smart_attributes"]["table"].as_array() {
            for attr in table {
                if let Some(id) = attr["id"].as_u64() {
                    let raw_val = attr["raw"]["value"].as_u64().unwrap_or(0);
                    match id {
                        // Temperature Attributes (194: Temperature_Celsius, 190: Airflow_Temperature)
                        194 | 190 => {
                            // Bits 0-7: Current Temperature
                            device.temperature = Some((raw_val & 0xFF) as f64);

                            // Seagate/WD often pack Min/Max in higher bytes
                            // Byte 2 (bits 16-23) is Min, Byte 4 (bits 32-39) is Max
                            if raw_val > 0xFFFF {
                                device.temperature_min = Some(((raw_val >> 16) & 0xFF) as f64);
                                device.temperature_max = Some(((raw_val >> 32) & 0xFF) as f64);
                            }
                        }

                        4 => device.start_stop_count = Some(raw_val),
                        5 => device.reallocated_sectors = Some(raw_val),
                        9 => device.power_on_hours = Some(raw_val),
                        12 => device.power_cycle_count = Some(raw_val),
                        193 => device.load_cycle_count = Some(raw_val),
                        197 => device.pending_sectors = Some(raw_val),
                        198 => device.uncorrectable_errors = Some(raw_val),
                        199 => device.crc_errors = Some(raw_val),

                        // SSD Wear Level (Life Remaining %)
                        // 231: SSD Life Left (Samsung/Kingston)
                        // 233: Media Wearout Indicator (Intel/Crucial)
                        // 202: Percentage Lifetime Used
                        231 | 233 | 202 => {
                            device.wear_level = Some(raw_val as f64);
                        }
                        _ => {}
                    }
                }
            }
        }

        device
    }
}

impl DataSource for SmartCtl {
    #[allow(clippy::manual_async_fn)]
    fn disk_temps(&self) -> impl Future<Output = anyhow::Result<SmartReports>> + Send {
        async move {
            let device_paths = self.scan_devices().await?;

            let mut tasks = FuturesUnordered::new();
            for path in device_paths {
                tasks.push(async move { self.query_device(&path).await.map_err(|e| (path, e)) });
            }

            let mut sata = Vec::new();
            let mut nvme = Vec::new();

            while let Some(result) = tasks.next().await {
                match result {
                    Ok(Some(DeviceReport::Sata(s))) => sata.push(s),
                    Ok(Some(DeviceReport::Nvme(n))) => nvme.push(n),
                    Ok(None) => {
                        tracing::debug!("Skipping device, because it's in low-power state");
                    }
                    Err((path, e)) => {
                        tracing::warn!(device = %path, error = %e, "Failed to query device SMART data");
                    }
                }
            }

            Ok(SmartReports {
                timestamp: Instant::now(),
                sata,
                nvme,
            })
        }
    }
}
