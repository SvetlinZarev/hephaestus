use crate::collector::{Collector, Metric};
use prometheus::{IntGaugeVec, Opts, Registry};
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

pub struct CpuFrequency {
    config: Config,
    system: Arc<Mutex<System>>,
    core_freq: IntGaugeVec,
}

impl CpuFrequency {
    pub fn new(config: Config, system: Arc<Mutex<System>>) -> anyhow::Result<Self> {
        let opts = Opts::new("system_cpu_core_frequency", "CPU core frequency");
        let core_freq = IntGaugeVec::new(opts, &["core"])?;

        Ok(Self {
            config,
            core_freq,
            system,
        })
    }
}

#[async_trait::async_trait]
impl Metric for CpuFrequency {
    fn name(&self) -> &'static str {
        "cpu-frequency"
    }

    fn enabled(&self) -> bool {
        self.config.enabled
    }

    async fn supported(&self) -> bool {
        true
    }

    fn register(&self, registry: &Registry) -> anyhow::Result<()> {
        registry.register(Box::new(self.core_freq.clone()))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Collector for CpuFrequency {
    async fn collect(&self) -> anyhow::Result<()> {
        let system = self.system.clone();
        let core_freq = self.core_freq.clone();

        tokio::task::spawn_blocking(move || match system.lock() {
            Err(error) => Err(anyhow::anyhow!(
                "Failed to refresh the CPU frequency statistics due to poisoned mutex: {}",
                error
            )),

            Ok(mut system) => {
                system.refresh_cpu_frequency();
                for cpu in system.cpus() {
                    core_freq
                        .with_label_values(&[cpu.name()])
                        .set(cpu.frequency() as i64);
                }

                Ok(())
            }
        })
        .await??;

        Ok(())
    }
}
