use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use prometheus::{IntGaugeVec, Opts, Registry};
use serde::{Deserialize, Serialize};

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
pub struct CpuFreqStats {
    pub cores: Vec<u64>,
}

pub trait DataSource {
    fn cpy_freq(&self) -> impl Future<Output = anyhow::Result<CpuFreqStats>> + Send;
}

#[derive(Clone)]
struct Metrics {
    core_freq: IntGaugeVec,
}

impl Metrics {
    fn register(registry: &Registry) -> anyhow::Result<Self> {
        let core_freq_opts = Opts::new(
            "system_cpu_core_frequency_hertz",
            "Current frequency of the CPU core in Hertz",
        );

        let core_freq = IntGaugeVec::new(core_freq_opts, &["core"])?;
        registry.register(Box::new(core_freq.clone()))?;

        Ok(Self { core_freq })
    }
}

pub struct CpuFrequency<T> {
    config: Config,
    data_source: T,
}

impl<T> CpuFrequency<T>
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

impl<T> Metric for CpuFrequency<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let metrics = Metrics::register(registry)?;
        Ok(Box::new(CpuFrequencyCollector::new(
            metrics,
            self.data_source,
        )))
    }
}

struct CpuFrequencyCollector<T> {
    metrics: Metrics,
    data_source: T,
}

impl<T> CpuFrequencyCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    pub fn new(metrics: Metrics, data_source: T) -> Self {
        Self {
            metrics,
            data_source,
        }
    }
}

#[async_trait::async_trait]
impl<T> Collector for CpuFrequencyCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self.data_source.cpy_freq().await?;
        for (core, &freq) in stats.cores.iter().enumerate() {
            self.metrics
                .core_freq
                .with_label_values(&[format!("{}", core)])
                .set(freq as i64);
        }

        Ok(())
    }
}
