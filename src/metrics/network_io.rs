use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use crate::metrics::util::{into_labels, maybe_counter, update_measurement_if};
use prometheus::Registry;
use prometheus::core::Desc;
use prometheus::proto::{LabelPair, MetricFamily};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub watch_interfaces: Option<Vec<String>>,
    pub ignore_interfaces: Option<Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            watch_interfaces: Some(vec!["bond0".to_owned(), "tailscale1".to_owned()]),
            ignore_interfaces: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InterfaceStats {
    pub interface: String,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
}

pub struct NetworkIoStats {
    pub timestamp: Instant,
    pub interfaces: Vec<InterfaceStats>,
}

pub trait DataSource {
    fn network_io(&self) -> impl Future<Output = anyhow::Result<NetworkIoStats>> + Send;
}
#[derive(Clone)]
pub struct Metrics {
    state: Arc<Mutex<Option<NetworkIoStats>>>,
    bytes_sent: Desc,
    bytes_received: Desc,
    packets_sent: Desc,
    packets_received: Desc,
}

impl Metrics {
    pub fn new(state: Arc<Mutex<Option<NetworkIoStats>>>) -> anyhow::Result<Self> {
        let labels = vec!["device".to_string()];
        Ok(Self {
            state,
            bytes_sent: Desc::new(
                "system_network_transmit_bytes_total".into(),
                "Total bytes sent".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            bytes_received: Desc::new(
                "system_network_receive_bytes_total".into(),
                "Total bytes received".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            packets_sent: Desc::new(
                "system_network_transmit_packets_total".into(),
                "Total packets sent".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            packets_received: Desc::new(
                "system_network_receive_packets_total".into(),
                "Total packets received".into(),
                labels,
                HashMap::new(),
            )?,
        })
    }

    pub fn register(&self, registry: &Registry) -> anyhow::Result<()> {
        registry.register(Box::new(self.clone()))?;
        Ok(())
    }

    fn make_labels(&self, device: &InterfaceStats) -> Vec<LabelPair> {
        into_labels(&[("device", &device.interface)])
    }
}

impl prometheus::core::Collector for Metrics {
    fn desc(&self) -> Vec<&Desc> {
        vec![
            &self.bytes_sent,
            &self.bytes_received,
            &self.packets_sent,
            &self.packets_received,
        ]
    }

    fn collect(&self) -> Vec<MetricFamily> {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stats) = guard.as_ref() else {
            return vec![];
        };

        let mut mf = vec![];
        for device in &stats.interfaces {
            let l = self.make_labels(device);
            maybe_counter(&mut mf, &self.bytes_sent, &l, Some(device.bytes_sent));
            maybe_counter(
                &mut mf,
                &self.bytes_received,
                &l,
                Some(device.bytes_received),
            );
            maybe_counter(&mut mf, &self.packets_sent, &l, Some(device.packets_sent));
            maybe_counter(
                &mut mf,
                &self.packets_received,
                &l,
                Some(device.packets_received),
            );
        }

        mf
    }
}
pub struct NetworkIo<T> {
    config: Config,
    data_source: T,
}
impl<T> NetworkIo<T>
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

impl<T> Metric for NetworkIo<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let collector = NetworkIoCollector::new(self.config, self.data_source);
        let measurements = collector.measurements();

        let metrics = Metrics::new(measurements)?;
        metrics.register(registry)?;

        Ok(Box::new(collector))
    }
}

struct NetworkIoCollector<T> {
    config: Config,
    measurement: Arc<Mutex<Option<NetworkIoStats>>>,
    data_source: T,
}

impl<T> NetworkIoCollector<T> {
    fn new(config: Config, data_source: T) -> Self {
        Self {
            config,
            data_source,
            measurement: Arc::new(Mutex::new(None)),
        }
    }

    fn measurements(&self) -> Arc<Mutex<Option<NetworkIoStats>>> {
        Arc::clone(&self.measurement)
    }

    fn should_collect(&self, interface_name: &str) -> bool {
        if let Some(watch) = &self.config.watch_interfaces {
            return watch.iter().any(|i| i == interface_name);
        }

        if let Some(ignore) = &self.config.ignore_interfaces {
            return !ignore.iter().any(|i| i == interface_name);
        }

        true
    }
}

#[async_trait::async_trait]
impl<T> Collector for NetworkIoCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    #[tracing::instrument(level = "debug", skip_all)]
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self
            .data_source
            .network_io()
            .await
            .map(|mut stats| {
                stats
                    .interfaces
                    .retain(|iface| self.should_collect(iface.interface.as_str()));
                stats
            })
            .inspect_err(|e| tracing::error!(error=?e, "Failed to collect network IO statistics"))
            .ok();

        update_measurement_if(&self.measurement, stats, |old, new| {
            old.timestamp < new.timestamp
        });

        Ok(())
    }
}
