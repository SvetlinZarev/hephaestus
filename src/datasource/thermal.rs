use crate::datasource::Reader;
use crate::metrics::thermal::{DataSource, ThermalStats, ThermalZoneStats};
use tokio::time::Instant;

const THERMAL_ZONE_DIR: &str = "/sys/class/thermal";

pub struct Thermal<R> {
    reader: R,
}

impl<R> Thermal<R>
where
    R: Reader,
{
    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R> DataSource for Thermal<R>
where
    R: Reader,
{
    #[tracing::instrument(level = "debug", skip_all)]
    async fn thermal(&self) -> anyhow::Result<ThermalStats> {
        let mut zones = Vec::new();

        let entries = self.reader.read_dir(THERMAL_ZONE_DIR).await?;
        for entry in entries {
            if !entry.name.starts_with("thermal_zone") {
                continue;
            }

            let zone_type = match self.reader.read_to_string(entry.path.join("type")).await {
                Ok(t) => t.trim().to_string(),
                Err(e) => {
                    tracing::debug!(zone=%entry.name, error=%e, "Skipping thermal zone: cannot read type");
                    continue;
                }
            };

            let temp_raw = match self.reader.read_to_string(entry.path.join("temp")).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::debug!(zone=%entry.name, error=%e, "Skipping thermal zone: cannot read temp");
                    continue;
                }
            };

            let Ok(millidegrees) = temp_raw.trim().parse::<i64>() else {
                tracing::debug!(zone=%entry.name, raw=%temp_raw.trim(), "Skipping thermal zone: cannot parse temp");
                continue;
            };

            zones.push(ThermalZoneStats {
                zone_type,
                temp_celsius: millidegrees as f64 / 1000.0,
            });
        }

        Ok(ThermalStats {
            timestamp: Instant::now(),
            zones,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::datasource::tests::HardcodedReader;
    use crate::datasource::thermal::Thermal;
    use crate::metrics::thermal::DataSource;

    #[tokio::test]
    async fn test_thermal_returns_error_when_dir_missing() {
        let reader = HardcodedReader::new();
        let ds = Thermal::new(reader);
        let result = ds.thermal().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_thermal_parses_zones() {
        let mut reader = HardcodedReader::new();
        reader.add_response("/sys/class/thermal/thermal_zone0/type", "x86_pkg_temp");
        reader.add_response("/sys/class/thermal/thermal_zone0/temp", "52000");
        reader.add_response("/sys/class/thermal/thermal_zone1/type", "acpitz");
        reader.add_response("/sys/class/thermal/thermal_zone1/temp", "41500");

        let ds = Thermal::new(reader);
        let stats = ds.thermal().await.unwrap();

        assert_eq!(stats.zones.len(), 2);

        let z0 = stats
            .zones
            .iter()
            .find(|z| z.zone_type == "x86_pkg_temp")
            .unwrap();
        assert!((z0.temp_celsius - 52.0).abs() < f64::EPSILON);

        let z1 = stats
            .zones
            .iter()
            .find(|z| z.zone_type == "acpitz")
            .unwrap();
        assert!((z1.temp_celsius - 41.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_thermal_skips_non_thermal_zone_entries() {
        let mut reader = HardcodedReader::new();
        // cooling_device should be skipped
        reader.add_response("/sys/class/thermal/cooling_device0/type", "Processor");
        reader.add_response("/sys/class/thermal/thermal_zone0/type", "acpitz");
        reader.add_response("/sys/class/thermal/thermal_zone0/temp", "30000");

        let ds = Thermal::new(reader);
        let stats = ds.thermal().await.unwrap();

        assert_eq!(stats.zones.len(), 1);
        assert_eq!(stats.zones[0].zone_type, "acpitz");
    }

    #[tokio::test]
    async fn test_thermal_skips_zone_with_missing_temp() {
        let mut reader = HardcodedReader::new();
        reader.add_response("/sys/class/thermal/thermal_zone0/type", "x86_pkg_temp");
        // no temp file for zone0

        let ds = Thermal::new(reader);
        let stats = ds.thermal().await.unwrap();

        assert!(stats.zones.is_empty());
    }

    #[tokio::test]
    async fn test_thermal_skips_zone_with_unparseable_temp() {
        let mut reader = HardcodedReader::new();
        reader.add_response("/sys/class/thermal/thermal_zone0/type", "x86_pkg_temp");
        reader.add_response("/sys/class/thermal/thermal_zone0/temp", "not_a_number");

        let ds = Thermal::new(reader);
        let stats = ds.thermal().await.unwrap();

        assert!(stats.zones.is_empty());
    }

    #[tokio::test]
    async fn test_thermal_negative_temperature() {
        let mut reader = HardcodedReader::new();
        reader.add_response("/sys/class/thermal/thermal_zone0/type", "acpitz");
        reader.add_response("/sys/class/thermal/thermal_zone0/temp", "-5000");

        let ds = Thermal::new(reader);
        let stats = ds.thermal().await.unwrap();

        assert_eq!(stats.zones.len(), 1);
        assert!((stats.zones[0].temp_celsius - (-5.0)).abs() < f64::EPSILON);
    }
}
