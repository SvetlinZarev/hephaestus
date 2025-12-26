use crate::collector::{Collector, Metric};
use prometheus::{IntGauge, Registry};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use sysinfo::{MemoryRefreshKind, System};

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

pub struct MemoryUsage {
    config: Config,
    system: Arc<Mutex<System>>,
    metrics: Metrics,
}

#[derive(Clone)]
struct Metrics {
    swap_free: IntGauge,
    swap_used: IntGauge,
    swap_total: IntGauge,

    mem_used: IntGauge,
    mem_free: IntGauge,
    mem_avail: IntGauge,
    mem_total: IntGauge,
}

impl MemoryUsage {
    pub fn new(config: Config, system: Arc<Mutex<System>>) -> anyhow::Result<Self> {
        let metrics = Metrics {
            swap_free: IntGauge::new("system_swap_free_bytes", "Amount of free swap memory")?,
            swap_used: IntGauge::new("system_swap_used_bytes", "Amount of used swap memory")?,
            swap_total: IntGauge::new("system_swap_total_bytes", "Amount of swap memory")?,
            mem_used: IntGauge::new("system_memory_used_bytes", "Amount of used system memory")?,
            mem_free: IntGauge::new(
                "system_memory_free_bytes",
                "Amount of available system memory",
            )?,
            mem_avail: IntGauge::new(
                "system_memory_available_bytes",
                "Amount of free system memory",
            )?,
            mem_total: IntGauge::new("system_memory_total_bytes", "Amount of system memory")?,
        };

        Ok(Self {
            config,
            system,
            metrics,
        })
    }
}

#[async_trait::async_trait]
impl Metric for MemoryUsage {
    fn name(&self) -> &'static str {
        "memory-usage"
    }

    fn enabled(&self) -> bool {
        self.config.enabled
    }

    async fn supported(&self) -> bool {
        true
    }

    fn register(&self, registry: &Registry) -> anyhow::Result<()> {
        if self.config.report_swap {
            registry.register(Box::new(self.metrics.swap_free.clone()))?;
            registry.register(Box::new(self.metrics.swap_used.clone()))?;
            registry.register(Box::new(self.metrics.swap_total.clone()))?;
        }

        registry.register(Box::new(self.metrics.mem_free.clone()))?;
        registry.register(Box::new(self.metrics.mem_used.clone()))?;
        registry.register(Box::new(self.metrics.mem_avail.clone()))?;
        registry.register(Box::new(self.metrics.mem_total.clone()))?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl Collector for MemoryUsage {
    async fn collect(&self) -> anyhow::Result<()> {
        let system = self.system.clone();
        let metrics = self.metrics.clone();
        let report_swap = self.config.report_swap;

        tokio::task::spawn_blocking(move || match system.lock() {
            Err(error) => Err(anyhow::anyhow!(
                "Failed to refresh the memory usage statistics due to poisoned mutex: {}",
                error
            )),

            Ok(mut system) => {
                system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());

                if report_swap {
                    system.refresh_memory_specifics(MemoryRefreshKind::nothing().with_swap());
                    metrics.swap_free.set(system.free_swap() as i64);
                    metrics.swap_used.set(system.used_swap() as i64);
                    metrics.swap_total.set(system.total_swap() as i64);
                }

                metrics.mem_free.set(system.free_memory() as i64);
                metrics.mem_used.set(system.used_memory() as i64);
                metrics.mem_avail.set(system.available_memory() as i64);
                metrics.mem_total.set(system.total_memory() as i64);

                Ok(())
            }
        })
        .await??;

        Ok(())
    }
}
