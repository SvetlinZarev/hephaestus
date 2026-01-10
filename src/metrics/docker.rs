use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use crate::metrics::util::{into_labels, maybe_counter, maybe_gauge, update_measurement_if};
use prometheus::core::Desc;
use prometheus::proto::{LabelPair, MetricFamily};
use prometheus::Registry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

#[derive(Debug, Clone)]
pub struct ContainerStats {
    pub name: String,
    pub cpu_usage_pct: f64,
    pub mem_usage_bytes: f64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct DockerStats {
    pub timestamp: Instant,
    pub containers: Vec<ContainerStats>,
}

pub trait DataSource {
    fn docker_stats(&self) -> impl Future<Output = anyhow::Result<DockerStats>> + Send;
}

#[derive(Clone)]
struct Metrics {
    state: Arc<Mutex<Option<DockerStats>>>,
    cpu_usage: Desc,
    mem_usage: Desc,
    net_rx: Desc,
    net_tx: Desc,
}

impl Metrics {
    pub fn new(state: Arc<Mutex<Option<DockerStats>>>) -> anyhow::Result<Self> {
        let labels = vec!["container".to_owned()];

        Ok(Self {
            state,
            cpu_usage: Desc::new(
                "docker_cpu_usage_percent".into(),
                "CPU usage percentage".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            mem_usage: Desc::new(
                "docker_memory_usage_bytes".into(),
                "Memory usage in bytes".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            net_rx: Desc::new(
                "docker_network_receive_bytes_total".into(),
                "Total bytes received".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            net_tx: Desc::new(
                "docker_network_transmit_bytes_total".into(),
                "Total bytes transmitted".into(),
                labels,
                HashMap::new(),
            )?,
        })
    }

    pub fn register(&self, registry: &Registry) -> anyhow::Result<()> {
        registry.register(Box::new(self.clone()))?;
        Ok(())
    }

    fn make_labels(&self, container: &ContainerStats) -> Vec<LabelPair> {
        into_labels(&[("container", &container.name)])
    }
}

impl prometheus::core::Collector for Metrics {
    fn desc(&self) -> Vec<&Desc> {
        vec![&self.cpu_usage, &self.mem_usage, &self.net_rx, &self.net_tx]
    }

    fn collect(&self) -> Vec<MetricFamily> {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stats) = guard.as_ref() else {
            return vec![];
        };

        let mut mf = Vec::with_capacity(stats.containers.len() * 4);
        for container in &stats.containers {
            let l = self.make_labels(container);
            maybe_gauge(&mut mf, &self.cpu_usage, &l, Some(container.cpu_usage_pct));
            maybe_gauge(
                &mut mf,
                &self.mem_usage,
                &l,
                Some(container.mem_usage_bytes),
            );
            maybe_counter(&mut mf, &self.net_rx, &l, Some(container.net_rx_bytes));
            maybe_counter(&mut mf, &self.net_tx, &l, Some(container.net_tx_bytes));
        }

        mf
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: true }
    }
}

pub struct Docker<T> {
    config: Config,
    data_source: T,
}

impl<T> Docker<T>
where
    T: DataSource + Send + Sync + 'static,
{
    pub fn new(config: Config, data_source: T) -> Self {
        Self {
            config,
            data_source,
        }
    }
}

impl<T> Metric for Docker<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let collector = DockerCollector::new(self.data_source);
        let measurements = collector.measurements();

        let metrics = Metrics::new(measurements)?;
        metrics.register(registry)?;

        Ok(Box::new(collector))
    }
}

struct DockerCollector<T> {
    measurement: Arc<Mutex<Option<DockerStats>>>,
    data_source: T,
}

impl<T> DockerCollector<T>
where
    T: DataSource,
{
    fn new(data_source: T) -> Self {
        Self {
            measurement: Arc::new(Mutex::new(None)),
            data_source,
        }
    }

    fn measurements(&self) -> Arc<Mutex<Option<DockerStats>>> {
        Arc::clone(&self.measurement)
    }
}

#[async_trait::async_trait]
impl<T> Collector for DockerCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self.data_source.docker_stats().await?;

        let guard = self.measurement.lock().unwrap_or_else(|e| e.into_inner());
        update_measurement_if(guard, stats, |old, new| old.timestamp < new.timestamp);

        Ok(())
    }
}
