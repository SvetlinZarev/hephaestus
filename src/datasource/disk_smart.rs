use crate::metrics::disk_smart::{DataSource, Device, DiskSmartStats, NvmeDevice, SataDevice};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde_json::Value;
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

    #[tracing::instrument(level = "trace", skip_all)]
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

    #[tracing::instrument(level = "trace", skip_all)]
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

    #[tracing::instrument(level = "trace", skip_all)]
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

    #[tracing::instrument(level = "trace", skip_all)]
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
    #[tracing::instrument(level = "debug", skip_all)]
    async fn disk_smart(&self) -> anyhow::Result<DiskSmartStats> {
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

        Ok(DiskSmartStats {
            timestamp: Instant::now(),
            sata,
            nvme,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::disk_smart::Device;
    use serde_json::json;

    fn test_device() -> Device {
        Device {
            device: "/dev/test".to_string(),
            model: "TestModel".to_string(),
            serial_number: "SN123".to_string(),
        }
    }

    #[test]
    fn test_parse_nvme_full() {
        let smart = SmartCtl::new();
        let json = json!({
            "nvme_smart_health_information_log": {
                "temperature": 35,
                "available_spare": 95,
                "percentage_used": 5,
                "data_units_read": 1_000_000,
                "data_units_written": 500_000,
                "host_reads": 2_000_000,
                "host_writes": 1_500_000,
                "power_on_hours": 1000,
                "unsafe_shutdowns": 2,
                "media_errors": 0
            }
        });

        let result = smart.parse_nvme(test_device(), &json);
        assert_eq!(result.temperature, Some(35.0));
        assert!((result.available_spare.unwrap() - 0.95).abs() < f64::EPSILON);
        assert!((result.percent_used.unwrap() - 0.05).abs() < f64::EPSILON);
        assert_eq!(result.data_units_read, Some(1_000_000));
        assert_eq!(result.data_units_written, Some(500_000));
        assert_eq!(result.host_reads, Some(2_000_000));
        assert_eq!(result.host_writes, Some(1_500_000));
        assert_eq!(result.power_on_hours, Some(1000));
        assert_eq!(result.unsafe_shutdowns, Some(2));
        assert_eq!(result.media_errors, Some(0));
    }

    #[test]
    fn test_parse_nvme_missing_health_log() {
        let smart = SmartCtl::new();
        let json = json!({});

        let result = smart.parse_nvme(test_device(), &json);
        assert!(result.temperature.is_none());
        assert!(result.available_spare.is_none());
        assert!(result.percent_used.is_none());
        assert!(result.data_units_read.is_none());
        assert!(result.data_units_written.is_none());
        assert!(result.host_reads.is_none());
        assert!(result.host_writes.is_none());
        assert!(result.power_on_hours.is_none());
        assert!(result.unsafe_shutdowns.is_none());
        assert!(result.media_errors.is_none());
    }

    #[test]
    fn test_parse_sata_all_attributes() {
        let smart = SmartCtl::new();
        let json = json!({
            "ata_smart_attributes": {
                "table": [
                    {"id": 4, "raw": {"value": 100}},
                    {"id": 5, "raw": {"value": 2}},
                    {"id": 9, "raw": {"value": 5000}},
                    {"id": 12, "raw": {"value": 50}},
                    {"id": 193, "raw": {"value": 300}},
                    {"id": 197, "raw": {"value": 1}},
                    {"id": 198, "raw": {"value": 3}},
                    {"id": 199, "raw": {"value": 7}},
                    {"id": 231, "raw": {"value": 95}},
                ]
            }
        });

        let result = smart.parse_sata(test_device(), &json);
        assert_eq!(result.start_stop_count, Some(100));
        assert_eq!(result.reallocated_sectors, Some(2));
        assert_eq!(result.power_on_hours, Some(5000));
        assert_eq!(result.power_cycle_count, Some(50));
        assert_eq!(result.load_cycle_count, Some(300));
        assert_eq!(result.pending_sectors, Some(1));
        assert_eq!(result.uncorrectable_errors, Some(3));
        assert_eq!(result.crc_errors, Some(7));
        assert_eq!(result.wear_level, Some(95.0));
    }

    #[test]
    fn test_parse_sata_temperature_simple() {
        let smart = SmartCtl::new();
        // raw_val <= 0xFFFF: only current temperature, no min/max
        let json = json!({
            "ata_smart_attributes": {
                "table": [
                    {"id": 194, "raw": {"value": 42}}
                ]
            }
        });

        let result = smart.parse_sata(test_device(), &json);
        assert_eq!(result.temperature, Some(42.0));
        assert!(result.temperature_min.is_none());
        assert!(result.temperature_max.is_none());
    }

    #[test]
    fn test_parse_sata_temperature_packed_min_max() {
        let smart = SmartCtl::new();
        // Packed format: current=35 (bits 0-7), min=20 (bits 16-23), max=55 (bits 32-39)
        let raw: u64 = 35 | (20 << 16) | (55u64 << 32);
        let json = json!({
            "ata_smart_attributes": {
                "table": [
                    {"id": 194, "raw": {"value": raw}}
                ]
            }
        });

        let result = smart.parse_sata(test_device(), &json);
        assert_eq!(result.temperature, Some(35.0));
        assert_eq!(result.temperature_min, Some(20.0));
        assert_eq!(result.temperature_max, Some(55.0));
    }

    #[test]
    fn test_parse_sata_empty_attributes() {
        let smart = SmartCtl::new();
        let json = json!({});

        let result = smart.parse_sata(test_device(), &json);
        assert!(result.temperature.is_none());
        assert!(result.start_stop_count.is_none());
        assert!(result.power_on_hours.is_none());
        assert!(result.wear_level.is_none());
    }

    #[test]
    fn test_parse_sata_wear_level_multiple_ids() {
        let smart = SmartCtl::new();
        // IDs 202, 231, 233 all map to wear_level — last one wins
        let json = json!({
            "ata_smart_attributes": {
                "table": [
                    {"id": 202, "raw": {"value": 80}},
                    {"id": 233, "raw": {"value": 90}},
                ]
            }
        });

        let result = smart.parse_sata(test_device(), &json);
        assert_eq!(result.wear_level, Some(90.0));
    }

    #[test]
    fn test_parse_sata_attr_190_also_sets_temperature() {
        let smart = SmartCtl::new();
        let json = json!({
            "ata_smart_attributes": {
                "table": [
                    {"id": 190, "raw": {"value": 38}}
                ]
            }
        });

        let result = smart.parse_sata(test_device(), &json);
        assert_eq!(result.temperature, Some(38.0));
    }
}
