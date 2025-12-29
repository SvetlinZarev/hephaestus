use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use prometheus::core::Desc;
use prometheus::proto::{LabelPair, MetricFamily, MetricType};
use prometheus::Registry;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceIoStats {
    pub device_name: String,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub read_ops: u64,
    pub write_ops: u64,
}

#[derive(Debug, Clone)]
pub struct DiskIoStats {
    pub timestamp: Instant,
    pub disks: Vec<DeviceIoStats>,
}

pub trait DataSource {
    fn disk_io(&self) -> impl Future<Output = anyhow::Result<DiskIoStats>> + Send;
}

#[derive(Clone)]
struct Metrics {
    state: Arc<Mutex<Option<DiskIoStats>>>,
    bytes_read: Desc,
    bytes_written: Desc,
    read_ops: Desc,
    write_ops: Desc,
}

impl Metrics {
    pub fn new(state: Arc<Mutex<Option<DiskIoStats>>>) -> anyhow::Result<Self> {
        let labels = vec!["device".to_owned()];

        let bytes_read = Desc::new(
            "system_disk_bytes_read_total".into(),
            "Total bytes read".into(),
            labels.clone(),
            HashMap::new(),
        )?;

        let bytes_written = Desc::new(
            "system_disk_bytes_written_total".into(),
            "Total bytes written".into(),
            labels.clone(),
            HashMap::new(),
        )?;

        let read_ops = Desc::new(
            "system_disk_read_ops_total".into(),
            "Total read ops".into(),
            labels.clone(),
            HashMap::new(),
        )?;

        let write_ops = Desc::new(
            "system_disk_write_ops_total".into(),
            "Total write ops".into(),
            labels.clone(),
            HashMap::new(),
        )?;

        Ok(Self {
            state,
            bytes_read,
            bytes_written,
            read_ops,
            write_ops,
        })
    }

    pub fn register(&self, registry: &Registry) -> anyhow::Result<()> {
        registry.register(Box::new(self.clone()))?;
        Ok(())
    }

    fn build_metric_family<F>(&self, desc: &Desc, stats: &DiskIoStats, extract: F) -> MetricFamily
    where
        F: Fn(&DeviceIoStats) -> u64,
    {
        let mut mf = MetricFamily::default();
        mf.set_name(desc.fq_name.clone());
        mf.set_help(desc.help.clone());
        mf.set_field_type(MetricType::COUNTER);

        let mut metrics = Vec::new();
        for dev in &stats.disks {
            let mut m = prometheus::proto::Metric::default();

            let mut lp = LabelPair::default();
            lp.set_name("device".into());
            lp.set_value(dev.device_name.clone());
            m.set_label(vec![lp].into());

            let mut c = prometheus::proto::Counter::default();
            c.set_value(extract(dev) as f64);
            m.set_counter(c);

            metrics.push(m);
        }

        mf.set_metric(metrics.into());
        mf
    }
}

impl prometheus::core::Collector for Metrics {
    fn desc(&self) -> Vec<&Desc> {
        vec![
            &self.bytes_read,
            &self.bytes_written,
            &self.read_ops,
            &self.write_ops,
        ]
    }

    fn collect(&self) -> Vec<MetricFamily> {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stats) = guard.as_ref() else {
            return vec![];
        };

        vec![
            self.build_metric_family(&self.bytes_read, stats, |d| d.bytes_read),
            self.build_metric_family(&self.bytes_written, stats, |d| d.bytes_written),
            self.build_metric_family(&self.read_ops, stats, |d| d.read_ops),
            self.build_metric_family(&self.write_ops, stats, |d| d.write_ops),
        ]
    }
}

pub struct DiskIo {
    config: Config,
}

impl DiskIo {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl<T> Metric<T> for DiskIo
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry, data_source: T) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let collector = DiskIoCollector::new(data_source);
        let measurements = collector.measurements();

        let metrics = Metrics::new(measurements)?;
        metrics.register(registry)?;

        Ok(Box::new(collector))
    }
}

struct DiskIoCollector<T> {
    measurement: Arc<Mutex<Option<DiskIoStats>>>,
    data_source: T,
}

impl<T> DiskIoCollector<T> {
    fn new(data_source: T) -> Self {
        Self {
            measurement: Arc::new(Mutex::new(None)),
            data_source,
        }
    }

    fn measurements(&self) -> Arc<Mutex<Option<DiskIoStats>>> {
        Arc::clone(&self.measurement)
    }

    fn should_collect(&self, device_name: &str) -> bool {
        !(device_name.starts_with("loop") || device_name.starts_with("zram"))
    }
}

#[async_trait::async_trait]
impl<T> Collector for DiskIoCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    async fn collect(&self) -> anyhow::Result<()> {
        let mut stats = self.data_source.disk_io().await?;
        stats
            .disks
            .retain(|disk| self.should_collect(&disk.device_name));

        let mut guard = self.measurement.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            None => *guard = Some(stats),
            Some(prev) => {
                if prev.timestamp < stats.timestamp {
                    *guard = Some(stats);
                }
            }
        }

        Ok(())
    }
}
