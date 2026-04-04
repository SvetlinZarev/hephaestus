use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use crate::metrics::util::{Measurement, maybe_counter};
use prometheus::Registry;
use prometheus::core::Desc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::time;

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
pub struct DatasetIoStats {
    pub pool: String,
    pub dataset: String,
    pub reads: u64,
    pub writes: u64,
    pub nread: u64,
    pub nwritten: u64,
}

#[derive(Debug, Clone)]
pub struct ZfsIoStats {
    pub timestamp: time::Instant,
    pub datasets: Vec<DatasetIoStats>,
}

pub trait DataSource {
    fn dataset_io(&self) -> impl Future<Output = anyhow::Result<ZfsIoStats>> + Send;
}

#[derive(Clone)]
struct Metrics {
    state: Measurement<ZfsIoStats>,
    reads: Desc,
    writes: Desc,
    nread: Desc,
    nwritten: Desc,
}

impl Metrics {
    pub fn new(state: Measurement<ZfsIoStats>) -> anyhow::Result<Self> {
        let labels = vec!["pool".to_owned(), "dataset".to_owned()];

        Ok(Self {
            state,
            reads: Desc::new(
                "zfs_dataset_reads_total".into(),
                "Total read operations".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            writes: Desc::new(
                "zfs_dataset_writes_total".into(),
                "Total write operations".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nread: Desc::new(
                "zfs_dataset_read_bytes_total".into(),
                "Total bytes read".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nwritten: Desc::new(
                "zfs_dataset_written_bytes_total".into(),
                "Total bytes written".into(),
                labels.clone(),
                HashMap::new(),
            )?,
        })
    }
}

impl prometheus::core::Collector for Metrics {
    fn desc(&self) -> Vec<&Desc> {
        vec![&self.reads, &self.writes, &self.nread, &self.nwritten]
    }

    fn collect(&self) -> Vec<prometheus::proto::MetricFamily> {
        self.state
            .read(|stats| {
                let mut mf = Vec::with_capacity(stats.datasets.len() * 4);
                for ds in &stats.datasets {
                    let l = crate::metrics::util::into_labels(&[
                        ("pool", &ds.pool),
                        ("dataset", &ds.dataset),
                    ]);

                    maybe_counter(&mut mf, &self.reads, &l, Some(ds.reads));
                    maybe_counter(&mut mf, &self.writes, &l, Some(ds.writes));
                    maybe_counter(&mut mf, &self.nread, &l, Some(ds.nread));
                    maybe_counter(&mut mf, &self.nwritten, &l, Some(ds.nwritten));
                }
                mf
            })
            .unwrap_or_default()
    }
}

pub struct ZfsDatasetIo<T> {
    config: Config,
    data_source: T,
}

impl<T> ZfsDatasetIo<T>
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

impl<T> Metric for ZfsDatasetIo<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let collector = ZfsDatasetIoCollector::new(self.data_source);
        let metrics = Metrics::new(collector.measurement.clone())?;
        registry.register(Box::new(metrics))?;

        Ok(Box::new(collector))
    }
}

struct ZfsDatasetIoCollector<T> {
    measurement: Measurement<ZfsIoStats>,
    data_source: T,
}

impl<T> ZfsDatasetIoCollector<T>
where
    T: DataSource,
{
    fn new(data_source: T) -> Self {
        Self {
            measurement: Measurement::new(),
            data_source,
        }
    }
}

#[async_trait::async_trait]
impl<T> Collector for ZfsDatasetIoCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    #[tracing::instrument(level = "debug", skip_all)]
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self
            .data_source
            .dataset_io()
            .await
            .inspect_err(|e| tracing::error!(error=?e, "Failed to collect ZFS dataset statistics"))
            .ok();

        self.measurement
            .update_if(stats, |old, new| old.timestamp < new.timestamp);

        Ok(())
    }
}
