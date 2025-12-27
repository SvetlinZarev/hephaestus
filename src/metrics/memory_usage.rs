use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use prometheus::{IntGauge, Registry};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub report_swap: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            report_swap: false,
        }
    }
}

#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub struct SwapStats {
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

#[derive(Default, Debug, Clone, Eq, PartialEq)]
pub struct RamStats {
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub available: u64,
    pub buffers: u64,
    pub cache: u64,
}

pub trait DataSource {
    fn swap(&self) -> impl Future<Output = anyhow::Result<SwapStats>> + Send;
    fn ram(&self) -> impl Future<Output = anyhow::Result<RamStats>> + Send;
}

#[derive(Debug, Clone)]
pub struct MemoryUsage {
    config: Config,
}

impl MemoryUsage {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl<T> Metric<T> for MemoryUsage
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry, data_source: T) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let mut swap_metrics = None;
        if self.config.report_swap {
            swap_metrics = Some(SwapMetrics::register(registry)?);
        }

        let ram_metrics = RamMetrics::register(registry)?;

        Ok(Box::new(MemoryUsageCollector::new(
            ram_metrics,
            swap_metrics,
            data_source,
        )))
    }
}

struct SwapMetrics {
    total: IntGauge,
    free: IntGauge,
    used: IntGauge,
}

impl SwapMetrics {
    fn register(registry: &Registry) -> anyhow::Result<Self> {
        let total = IntGauge::new(
            "system_swap_total_bytes",
            "Total amount of swap space available",
        )?;
        registry.register(Box::new(total.clone()))?;

        let free = IntGauge::new(
            "system_swap_free_bytes",
            "Amount of swap space currently unused",
        )?;
        registry.register(Box::new(free.clone()))?;

        let used = IntGauge::new(
            "system_swap_used_bytes",
            "Amount of swap space currently in use",
        )?;
        registry.register(Box::new(used.clone()))?;

        Ok(Self { total, free, used })
    }
}

#[derive(Clone)]
struct RamMetrics {
    total: IntGauge,
    used: IntGauge,
    free: IntGauge,
    avail: IntGauge,
    buffers: IntGauge,
    cache: IntGauge,
}

impl RamMetrics {
    fn register(registry: &Registry) -> anyhow::Result<Self> {
        let total = IntGauge::new(
            "system_memory_total_bytes",
            "Total physical RAM installed on the system",
        )?;
        registry.register(Box::new(total.clone()))?;

        let used = IntGauge::new(
            "system_memory_used_bytes",
            "Amount of memory currently used by programs (Non-reclaimable)",
        )?;
        registry.register(Box::new(used.clone()))?;

        let free = IntGauge::new(
            "system_memory_free_bytes",
            "Amount of memory that is completely unused (does not include cache/buffers)",
        )?;
        registry.register(Box::new(free.clone()))?;

        let avail = IntGauge::new(
            "system_memory_available_bytes",
            "Estimate of how much memory is available for starting new applications without swapping",
        )?;
        registry.register(Box::new(avail.clone()))?;

        let buffers = IntGauge::new(
            "system_memory_buffers_bytes",
            "Memory used by kernel buffers (metadata/raw block storage)",
        )?;
        registry.register(Box::new(buffers.clone()))?;

        let cache = IntGauge::new(
            "system_memory_cache_bytes",
            "Memory used by the page cache and reclaimable slab objects",
        )?;
        registry.register(Box::new(cache.clone()))?;

        Ok(Self {
            total,
            used,
            free,
            avail,
            buffers,
            cache,
        })
    }
}

struct MemoryUsageCollector<T> {
    ram_metrics: RamMetrics,
    swap_metrics: Option<SwapMetrics>,
    data_source: T,
}

impl<T> MemoryUsageCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn new(ram_metrics: RamMetrics, swap_metrics: Option<SwapMetrics>, data_source: T) -> Self {
        Self {
            ram_metrics,
            swap_metrics,
            data_source,
        }
    }
}

#[async_trait::async_trait]
impl<T> Collector for MemoryUsageCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    async fn collect(&self) -> anyhow::Result<()> {
        if let Some(swap_metrics) = &self.swap_metrics {
            let stats = self.data_source.swap().await?;
            swap_metrics.free.set(stats.free as i64);
            swap_metrics.used.set(stats.used as i64);
            swap_metrics.total.set(stats.total as i64);
        }

        let stats = self.data_source.ram().await?;
        self.ram_metrics.free.set(stats.free as i64);
        self.ram_metrics.used.set(stats.used as i64);
        self.ram_metrics.total.set(stats.total as i64);
        self.ram_metrics.avail.set(stats.available as i64);
        self.ram_metrics.buffers.set(stats.buffers as i64);
        self.ram_metrics.cache.set(stats.cache as i64);

        Ok(())
    }
}
