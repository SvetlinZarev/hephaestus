use crate::collector::{Collector, Metric};
use prometheus::{Gauge, GaugeVec, Opts, Registry};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use sysinfo::System;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: true }
    }
}

pub struct CpuUsageCollector {
    config: Config,
    core_usage: GaugeVec,
    total_usage: Gauge,
    system: Arc<Mutex<System>>,
}

impl CpuUsageCollector {
    pub fn new(config: Config, system: Arc<Mutex<System>>) -> anyhow::Result<Self> {
        let opts = Opts::new("system_cpu_core_usage", "CPU usage percentage per core");
        let core_usage = GaugeVec::new(opts, &["core"])?;
        let total_usage = Gauge::new("system_cpu_total_usage", "CPU usage across all cores")?;

        Ok(Self {
            config,
            core_usage,
            total_usage,
            system,
        })
    }
}

#[async_trait::async_trait]
impl Metric for CpuUsageCollector {
    fn name(&self) -> &'static str {
        "cpu-usage"
    }

    fn enabled(&self) -> bool {
        self.config.enabled
    }

    async fn supported(&self) -> bool {
        true
    }

    fn register(&self, registry: &Registry) -> anyhow::Result<()> {
        registry.register(Box::new(self.core_usage.clone()))?;
        registry.register(Box::new(self.total_usage.clone()))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Collector for CpuUsageCollector {
    async fn collect(&self) -> anyhow::Result<()> {
        let system = self.system.clone();
        let total_usage = self.total_usage.clone();
        let core_usage = self.core_usage.clone();

        tokio::task::spawn_blocking(move || match system.lock() {
            Err(error) => {
                tracing::error!(
                    "Failed to refresh the CPU usage, because the mutex is poisoned: {}",
                    error
                );
            }

            Ok(mut system) => {
                system.refresh_cpu_usage();
                let cpus = system.cpus();

                total_usage.set(system.global_cpu_usage() as f64);
                for cpu in cpus.iter() {
                    core_usage
                        .with_label_values(&[cpu.name()])
                        .set(cpu.cpu_usage() as f64);
                }
            }
        })
        .await?;

        Ok(())
    }
}
