use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use crate::metrics::util::update_measurement_if;
use prometheus::Registry;
use prometheus::core::Desc;
use prometheus::proto::{LabelPair, MetricFamily, MetricType};
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

    pub estimated_runtime: Option<f64>,
    pub battery_level: Option<f64>,
    pub load: Option<f64>,

    pub input_voltage: Option<f64>,
    pub output_voltage: Option<f64>,

    pub nominal_apparent_power: Option<f64>,
    pub nominal_real_power: Option<f64>,

    pub apparent_power: Option<f64>,
    pub real_power: Option<f64>,
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
    nominal_apparent_power: Desc,
    nominal_real_power: Desc,
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
        let nominal_apparent_power = Desc::new(
            "system_ups_nominal_apparent_power_va".into(),
            "Nominal apparent power".into(),
            labels.clone(),
            HashMap::new(),
        )?;
        let nominal_real_power = Desc::new(
            "system_ups_nominal_real_power_watts".into(),
            "Nominal real power".into(),
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
            nominal_apparent_power,
            nominal_real_power,
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
                m.set_label(vec![lp]);

                let mut g = prometheus::proto::Gauge::default();
                g.set_value(val);
                m.set_gauge(g);
                metrics.push(m);
            }
        }
        mf.set_metric(metrics);
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

        mf.push(self.build_metric_family(&self.runtime, stats, |u| u.estimated_runtime));
        mf.push(self.build_metric_family(&self.battery_level, stats, |u| u.battery_level));
        mf.push(
            self.build_metric_family(&self.nominal_apparent_power, stats, |u| {
                u.nominal_apparent_power
            }),
        );
        mf.push(
            self.build_metric_family(&self.nominal_real_power, stats, |u| u.nominal_real_power),
        );
        mf.push(self.build_metric_family(&self.apparent_power, stats, |u| u.apparent_power));
        mf.push(self.build_metric_family(&self.real_power, stats, |u| u.real_power));
        mf.push(self.build_metric_family(&self.load, stats, |u| u.load));
        mf.push(self.build_metric_family(&self.input_voltage, stats, |u| u.input_voltage));
        mf.push(self.build_metric_family(&self.output_voltage, stats, |u| u.output_voltage));

        mf
    }
}

pub struct Ups<T> {
    config: Config,
    data_source: T,
}

impl<T> Ups<T>
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

impl<T> Metric for Ups<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let collector = UpsCollector::new(self.data_source);
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
    #[tracing::instrument(level = "debug", skip_all)]
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self.data_source.ups_stats().await?;
        let guard = self.measurements.lock().unwrap_or_else(|e| e.into_inner());
        update_measurement_if(guard, stats, |old, new| old.timestamp < new.timestamp);
        Ok(())
    }
}
