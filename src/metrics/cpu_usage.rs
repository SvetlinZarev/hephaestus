use crate::domain::{Collector, Metric};

use crate::metrics::no_operation::NoOpCollector;
use prometheus::{Gauge, GaugeVec, Opts, Registry};
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

pub trait DataSource {
    fn cpu_usage(&self) -> impl Future<Output = anyhow::Result<CpuUsageStats>> + Send;
}

#[derive(Debug, Clone)]
pub struct CpuUsageStats {
    pub total_usage: f64,
    pub total_breakdown: CoreStats,
    pub cores: Vec<CoreUsageStats>,
}

#[derive(Debug, Clone)]
pub struct CoreUsageStats {
    pub core: usize,
    pub total_usage: f64,
    pub breakdown: CoreStats,
}

#[derive(Debug, Clone, Default)]
pub struct CoreStats {
    pub user: f64,
    pub nice: f64,
    pub system: f64,
    pub idle: f64,
    pub iowait: f64,
    pub irq: f64,
    pub softirq: f64,
    pub steal: f64,
    pub guest: f64,
    pub guest_nice: f64,
}

pub struct CpuUsage<T> {
    config: Config,
    data_source: T,
}

impl<T> CpuUsage<T>
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

impl<T> Metric for CpuUsage<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let metrics = Metrics::register(registry)?;
        Ok(Box::new(CpuUsageCollector::new(metrics, self.data_source)))
    }
}

#[derive(Clone)]
struct Metrics {
    total_usage: Gauge,
    total_breakdown: GaugeVec, // Labels: ["type"]

    // Per-core metrics
    core_usage: GaugeVec,     // Labels: ["core"]
    core_breakdown: GaugeVec, // Labels: ["core", "type"]
}

impl Metrics {
    fn register(registry: &Registry) -> anyhow::Result<Self> {
        let total_usage = Gauge::new("system_cpu_usage_ratio", "Overall CPU usage ratio")?;
        registry.register(Box::new(total_usage.clone()))?;

        let total_breakdown = GaugeVec::new(
            Opts::new(
                "system_cpu_time_type_ratio",
                "Overall CPU time breakdown by type",
            ),
            &["type"],
        )?;
        registry.register(Box::new(total_breakdown.clone()))?;

        let core_usage = GaugeVec::new(
            Opts::new("system_cpu_core_usage_ratio", "Per-core CPU usage ratio"),
            &["core"],
        )?;
        registry.register(Box::new(core_usage.clone()))?;

        let core_breakdown = GaugeVec::new(
            Opts::new(
                "system_cpu_core_time_type_ratio",
                "Per-core CPU time breakdown by type",
            ),
            &["core", "type"],
        )?;
        registry.register(Box::new(core_breakdown.clone()))?;

        Ok(Self {
            total_usage,
            total_breakdown,
            core_usage,
            core_breakdown,
        })
    }
}
struct CpuUsageCollector<T> {
    metrics: Metrics,
    data_source: T,
}

impl<T> CpuUsageCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn new(metrics: Metrics, data_source: T) -> Self {
        Self {
            metrics,
            data_source,
        }
    }

    fn update_gauge(&self, gauge_vec: &GaugeVec, stats: &CoreStats, core_label: Option<&str>) {
        let set_val = |time_type: &str, value: f64| {
            if let Some(core) = core_label {
                gauge_vec.with_label_values(&[core, time_type]).set(value);
            } else {
                gauge_vec.with_label_values(&[time_type]).set(value);
            }
        };

        set_val("user", stats.user);
        set_val("nice", stats.nice);
        set_val("system", stats.system);
        set_val("idle", stats.idle);
        set_val("iowait", stats.iowait);
        set_val("irq", stats.irq);
        set_val("softirq", stats.softirq);
        set_val("steal", stats.steal);
        set_val("guest", stats.guest);
        set_val("guest_nice", stats.guest_nice);
    }
}

#[async_trait::async_trait]
impl<T> Collector for CpuUsageCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    #[tracing::instrument(level = "debug", skip_all)]
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self.data_source.cpu_usage().await?;

        self.metrics.total_usage.set(stats.total_usage);
        self.update_gauge(&self.metrics.total_breakdown, &stats.total_breakdown, None);

        for core_stat in stats.cores {
            let core_label = core_stat.core.to_string();

            self.metrics
                .core_usage
                .with_label_values(&[&core_label])
                .set(core_stat.total_usage);

            self.update_gauge(
                &self.metrics.core_breakdown,
                &core_stat.breakdown,
                Some(&core_label),
            );
        }

        Ok(())
    }
}
