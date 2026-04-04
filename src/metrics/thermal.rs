use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use crate::metrics::util::{Measurement, into_labels, maybe_gauge};
use prometheus::Registry;
use prometheus::core::Desc;
use prometheus::proto::MetricFamily;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone)]
pub struct ThermalZoneStats {
    pub zone_type: String,
    pub temp_celsius: f64,
}

#[derive(Debug, Clone)]
pub struct ThermalStats {
    pub timestamp: Instant,
    pub zones: Vec<ThermalZoneStats>,
}

pub trait DataSource {
    fn thermal(&self) -> impl Future<Output = anyhow::Result<ThermalStats>> + Send;
}

#[derive(Clone)]
struct Metrics {
    state: Measurement<ThermalStats>,
    temp: Desc,
}

impl Metrics {
    fn new(state: Measurement<ThermalStats>) -> anyhow::Result<Self> {
        Ok(Self {
            state,
            temp: Desc::new(
                "system_thermal_zone_celsius".into(),
                "Current temperature of the thermal zone".into(),
                vec!["zone".into()],
                HashMap::new(),
            )?,
        })
    }
}

impl prometheus::core::Collector for Metrics {
    fn desc(&self) -> Vec<&Desc> {
        vec![&self.temp]
    }

    fn collect(&self) -> Vec<MetricFamily> {
        self.state
            .read(|stats| {
                let mut families = Vec::new();
                for zone in &stats.zones {
                    let l = into_labels(&[("zone", &zone.zone_type)]);
                    maybe_gauge(&mut families, &self.temp, &l, Some(zone.temp_celsius));
                }
                families
            })
            .unwrap_or_default()
    }
}

pub struct Thermal<T> {
    config: Config,
    data_source: T,
}

impl<T> Thermal<T>
where
    T: DataSource,
{
    pub fn new(config: Config, data_source: T) -> Self {
        Self {
            config,
            data_source,
        }
    }
}

impl<T> Metric for Thermal<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let collector = ThermalCollector::new(self.data_source);
        registry.register(Box::new(collector.metrics()?))?;

        Ok(Box::new(collector))
    }
}

struct ThermalCollector<T> {
    measurement: Measurement<ThermalStats>,
    data_source: T,
}

impl<T> ThermalCollector<T>
where
    T: DataSource,
{
    fn new(data_source: T) -> Self {
        Self {
            measurement: Measurement::new(),
            data_source,
        }
    }

    fn metrics(&self) -> anyhow::Result<Metrics> {
        Metrics::new(self.measurement.clone())
    }
}

#[async_trait::async_trait]
impl<T> Collector for ThermalCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    #[tracing::instrument(level = "debug", skip_all)]
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self
            .data_source
            .thermal()
            .await
            .inspect_err(|e| tracing::error!(error=?e, "Failed to collect thermal zone statistics"))
            .ok();

        self.measurement
            .update_if(stats, |old, new| old.timestamp < new.timestamp);

        Ok(())
    }
}
