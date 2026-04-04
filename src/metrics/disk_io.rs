use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use crate::metrics::util::{Measurement, into_labels, maybe_counter};
use prometheus::Registry;
use prometheus::core::Desc;
use prometheus::proto::{LabelPair, MetricFamily};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    state: Measurement<DiskIoStats>,
    bytes_read: Desc,
    bytes_written: Desc,
    read_ops: Desc,
    write_ops: Desc,
}

impl Metrics {
    pub fn new(state: Measurement<DiskIoStats>) -> anyhow::Result<Self> {
        let labels = vec!["device".to_owned()];

        let bytes_read = Desc::new(
            "system_disk_read_bytes_total".into(),
            "Total bytes read".into(),
            labels.clone(),
            HashMap::new(),
        )?;

        let bytes_written = Desc::new(
            "system_disk_written_bytes_total".into(),
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

    fn make_labels(&self, device: &DeviceIoStats) -> Vec<LabelPair> {
        into_labels(&[("device", &device.device_name)])
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
        self.state
            .read(|stats| {
                let mut mf = Vec::with_capacity(stats.disks.len());
                for device in &stats.disks {
                    let l = self.make_labels(device);
                    maybe_counter(&mut mf, &self.bytes_read, &l, Some(device.bytes_read));
                    maybe_counter(&mut mf, &self.bytes_written, &l, Some(device.bytes_written));
                    maybe_counter(&mut mf, &self.read_ops, &l, Some(device.read_ops));
                    maybe_counter(&mut mf, &self.write_ops, &l, Some(device.write_ops));
                }
                mf
            })
            .unwrap_or_default()
    }
}

pub struct DiskIo<T> {
    config: Config,
    data_source: T,
}

impl<T> DiskIo<T>
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

impl<T> Metric for DiskIo<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let collector = DiskIoCollector::new(self.data_source);

        let metrics = Metrics::new(collector.measurement.clone())?;
        registry.register(Box::new(metrics))?;

        Ok(Box::new(collector))
    }
}

struct DiskIoCollector<T> {
    measurement: Measurement<DiskIoStats>,
    data_source: T,
}

impl<T> DiskIoCollector<T>
where
    T: DataSource,
{
    fn new(data_source: T) -> Self {
        Self {
            measurement: Measurement::new(),
            data_source,
        }
    }

    fn should_collect(&self, device_name: &str) -> bool {
        if device_name.starts_with("loop") || device_name.starts_with("zram") {
            return false;
        }

        if device_name.starts_with("nvme") && device_name.rsplit_once('p').is_some() {
            // Ignore NVMe partitions
            return false;
        }

        if device_name.starts_with("sd")
            && device_name.len() > 3
            && device_name.as_bytes().last().unwrap().is_ascii_digit()
        {
            // Ignore HDD partitions (i.e. sda1, sda2, etc)
            return false;
        }

        true
    }
}

#[async_trait::async_trait]
impl<T> Collector for DiskIoCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    #[tracing::instrument(level = "debug", skip_all)]
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self
            .data_source
            .disk_io()
            .await
            .map(|mut stats| {
                stats
                    .disks
                    .retain(|disk| self.should_collect(&disk.device_name));
                stats
            })
            .inspect_err(|err| tracing::error!(error=?err, "Failed to collect disk IO statistics"))
            .ok();

        self.measurement
            .update_if(stats, |old, new| old.timestamp < new.timestamp);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopDataSource;

    impl DataSource for NoopDataSource {
        async fn disk_io(&self) -> anyhow::Result<DiskIoStats> {
            unimplemented!()
        }
    }

    fn create_collector() -> DiskIoCollector<NoopDataSource> {
        DiskIoCollector {
            measurement: Measurement::new(),
            data_source: NoopDataSource,
        }
    }

    #[test]
    fn test_should_collect_loop_device() {
        let collector = create_collector();
        assert!(!collector.should_collect("loop0"));
        assert!(!collector.should_collect("loop12"));
    }

    #[test]
    fn test_should_collect_zram_device() {
        let collector = create_collector();
        assert!(!collector.should_collect("zram0"));
    }

    #[test]
    fn test_should_collect_nvme_whole_disk() {
        let collector = create_collector();
        assert!(collector.should_collect("nvme0n1"));
    }

    #[test]
    fn test_should_collect_nvme_partition() {
        let collector = create_collector();
        assert!(!collector.should_collect("nvme0n1p1"));
        assert!(!collector.should_collect("nvme0n1p2"));
    }

    #[test]
    fn test_should_collect_sda_whole_disk() {
        let collector = create_collector();
        assert!(collector.should_collect("sda"));
        assert!(collector.should_collect("sdb"));
    }

    #[test]
    fn test_should_collect_sda_partition() {
        let collector = create_collector();
        assert!(!collector.should_collect("sda1"));
        assert!(!collector.should_collect("sdb2"));
    }

    #[test]
    fn test_should_collect_dm_device() {
        let collector = create_collector();
        assert!(collector.should_collect("dm-0"));
    }
}
