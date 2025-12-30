use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use prometheus::core::Desc;
use prometheus::proto::{LabelPair, MetricFamily, MetricType};
use prometheus::Registry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
pub struct UpsStats {
    pub timestamp: Instant,
    pub devices: Vec<UpsDeviceStats>,
}

#[derive(Debug, Clone)]
pub struct UpsDeviceStats {
    pub device_name: String,
    pub estimated_runtime: f64,
    pub battery_level: f64,
    pub input_voltage: Option<f64>,
    pub output_voltage: Option<f64>,
    pub apparent_power: f64,
    pub real_power: f64,
    pub load: f64,
}

pub trait DataSource {
    fn ups_stats(&self) -> impl Future<Output = anyhow::Result<UpsStats>> + Send;
}

#[derive(Clone)]
pub struct Metrics {
    state: Arc<Mutex<Option<UpsStats>>>,
    runtime: Desc,
    battery_level: Desc,
    input_voltage: Desc,
    output_voltage: Desc,
    apparent_power: Desc,
    real_power: Desc,
    load: Desc,
}

impl Metrics {
    pub fn new(state: Arc<Mutex<Option<UpsStats>>>) -> anyhow::Result<Self> {
        let labels = vec!["ups".to_string()];
        let runtime = Desc::new(
            "system_ups_runtime_seconds".into(),
            "Estimated battery runtime".into(),
            labels.clone(),
            HashMap::new(),
        )?;

        let battery_level = Desc::new(
            "system_ups_battery_level_percent".into(),
            "Battery charge level".into(),
            labels.clone(),
            HashMap::new(),
        )?;
        let input_voltage = Desc::new(
            "system_ups_input_voltage".into(),
            "Input line voltage".into(),
            labels.clone(),
            HashMap::new(),
        )?;
        let output_voltage = Desc::new(
            "system_ups_output_voltage".into(),
            "Output line voltage".into(),
            labels.clone(),
            HashMap::new(),
        )?;
        let apparent_power = Desc::new(
            "system_ups_apparent_power_va".into(),
            "Apparent power draw".into(),
            labels.clone(),
            HashMap::new(),
        )?;
        let real_power = Desc::new(
            "system_ups_real_power_watts".into(),
            "Real power draw".into(),
            labels.clone(),
            HashMap::new(),
        )?;
        let load = Desc::new(
            "system_ups_load_percent".into(),
            "UPS load percentage".into(),
            labels,
            HashMap::new(),
        )?;

        Ok(Self {
            state,
            runtime,
            battery_level,
            input_voltage,
            output_voltage,
            apparent_power,
            real_power,
            load,
        })
    }

    pub fn register(&self, registry: &Registry) -> anyhow::Result<()> {
        registry.register(Box::new(self.clone()))?;
        Ok(())
    }

    fn build_metric_family<F>(&self, desc: &Desc, stats: &UpsStats, extract: F) -> MetricFamily
    where
        F: Fn(&UpsDeviceStats) -> Option<f64>,
    {
        let mut mf = MetricFamily::default();
        mf.set_name(desc.fq_name.clone());
        mf.set_help(desc.help.clone());
        mf.set_field_type(MetricType::GAUGE);

        let mut metrics = Vec::new();
        for ups in &stats.devices {
            if let Some(val) = extract(ups) {
                let mut m = prometheus::proto::Metric::default();
                let mut lp = LabelPair::default();
                lp.set_name("ups".into());
                lp.set_value(ups.device_name.clone());
                m.set_label(vec![lp].into());

                let mut g = prometheus::proto::Gauge::default();
                g.set_value(val);
                m.set_gauge(g);
                metrics.push(m);
            }
        }
        mf.set_metric(metrics.into());
        mf
    }
}

impl prometheus::core::Collector for Metrics {
    fn desc(&self) -> Vec<&Desc> {
        vec![
            &self.runtime,
            &self.battery_level,
            &self.input_voltage,
            &self.output_voltage,
            &self.apparent_power,
            &self.real_power,
            &self.load,
        ]
    }

    fn collect(&self) -> Vec<MetricFamily> {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stats) = guard.as_ref() else {
            return vec![];
        };

        let mut mf = Vec::new();

        mf.push(self.build_metric_family(&self.runtime, stats, |u| Some(u.estimated_runtime)));
        mf.push(self.build_metric_family(&self.battery_level, stats, |u| Some(u.battery_level)));
        mf.push(self.build_metric_family(&self.apparent_power, stats, |u| Some(u.apparent_power)));
        mf.push(self.build_metric_family(&self.real_power, stats, |u| Some(u.real_power)));
        mf.push(self.build_metric_family(&self.load, stats, |u| Some(u.load)));
        mf.push(
            self.build_metric_family(&self.input_voltage, stats, |u| u.input_voltage.map(|v| v)),
        );
        mf.push(
            self.build_metric_family(&self.output_voltage, stats, |u| u.output_voltage.map(|v| v)),
        );

        mf
    }
}

pub struct Ups {
    config: Config,
}

impl Ups {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl<T> Metric<T> for Ups
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry, data_source: T) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let collector = UpsCollector::new(data_source);
        let measurements = collector.measurements();

        let metrics = Metrics::new(measurements)?;
        metrics.register(registry)?;

        Ok(Box::new(collector))
    }
}

struct UpsCollector<T> {
    data_source: T,
    measurements: Arc<Mutex<Option<UpsStats>>>,
}

impl<T> UpsCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    pub fn new(data_source: T) -> Self {
        Self {
            data_source,
            measurements: Arc::new(Mutex::new(None)),
        }
    }

    fn measurements(&self) -> Arc<Mutex<Option<UpsStats>>> {
        self.measurements.clone()
    }
}

#[async_trait::async_trait]
impl<T> Collector for UpsCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self.data_source.ups_stats().await?;
        let mut guard = self.measurements.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() || guard.as_ref().unwrap().timestamp < stats.timestamp {
            *guard = Some(stats);
        }

        Ok(())
    }
}
