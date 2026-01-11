use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use crate::metrics::util::{maybe_counter, maybe_gauge, update_measurement_if};
use prometheus::Registry;
use prometheus::core::Desc;
use prometheus::proto::MetricFamily;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
pub struct ArcStats {
    pub timestamp: Instant,
    pub hits: u64,
    pub misses: u64,
    pub size: u64,
    pub target_size: u64,
    pub max_size: u64,
}

pub trait DataSource {
    fn arc_stats(&self) -> impl Future<Output = anyhow::Result<ArcStats>> + Send;
}

#[derive(Clone)]
struct Metrics {
    state: Arc<Mutex<Option<ArcStats>>>,
    hits: Desc,
    misses: Desc,
    size: Desc,
    target_size: Desc,
    max_size: Desc,
}

impl Metrics {
    pub fn new(state: Arc<Mutex<Option<ArcStats>>>) -> anyhow::Result<Self> {
        let labels = HashMap::new();

        Ok(Self {
            state,
            hits: Desc::new(
                "zfs_arc_hits_total".into(),
                "Total ARC hits".into(),
                vec![],
                labels.clone(),
            )?,
            misses: Desc::new(
                "zfs_arc_misses_total".into(),
                "Total ARC misses".into(),
                vec![],
                labels.clone(),
            )?,
            size: Desc::new(
                "zfs_arc_size_bytes".into(),
                "Current size of ARC".into(),
                vec![],
                labels.clone(),
            )?,
            target_size: Desc::new(
                "zfs_arc_target_size_bytes".into(),
                "Target size of ARC".into(),
                vec![],
                labels.clone(),
            )?,
            max_size: Desc::new(
                "zfs_arc_max_size_bytes".into(),
                "Maximum size of ARC".into(),
                vec![],
                labels.clone(),
            )?,
        })
    }

    pub fn register(&self, registry: &Registry) -> anyhow::Result<()> {
        registry.register(Box::new(self.clone()))?;
        Ok(())
    }
}

pub struct ZfsArc<T> {
    config: Config,
    data_source: T,
}

impl<T> ZfsArc<T>
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

impl<T> Metric for ZfsArc<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let collector = ZfsCollector::new(self.data_source);
        let metrics = Metrics::new(collector.measurements())?;
        metrics.register(registry)?;

        Ok(Box::new(collector))
    }
}

struct ZfsCollector<T> {
    measurement: Arc<Mutex<Option<ArcStats>>>,
    data_source: T,
}

impl<T> ZfsCollector<T>
where
    T: DataSource,
{
    fn new(data_source: T) -> Self {
        Self {
            measurement: Arc::new(Mutex::new(None)),
            data_source,
        }
    }

    fn measurements(&self) -> Arc<Mutex<Option<ArcStats>>> {
        Arc::clone(&self.measurement)
    }
}

#[async_trait::async_trait]
impl<T> Collector for ZfsCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    #[tracing::instrument(level = "debug", skip_all)]
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self.data_source.arc_stats().await?;
        let guard = self.measurement.lock().unwrap_or_else(|e| e.into_inner());
        update_measurement_if(guard, stats, |old, new| old.timestamp < new.timestamp);
        Ok(())
    }
}

impl prometheus::core::Collector for Metrics {
    fn desc(&self) -> Vec<&Desc> {
        vec![
            &self.hits,
            &self.misses,
            &self.size,
            &self.target_size,
            &self.max_size,
        ]
    }

    fn collect(&self) -> Vec<MetricFamily> {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stats) = guard.as_ref() else {
            return vec![];
        };

        let mut mf = Vec::new();
        let l = vec![];

        maybe_counter(&mut mf, &self.hits, &l, Some(stats.hits));
        maybe_counter(&mut mf, &self.misses, &l, Some(stats.misses));
        maybe_gauge(&mut mf, &self.size, &l, Some(stats.size as f64));
        maybe_gauge(
            &mut mf,
            &self.target_size,
            &l,
            Some(stats.target_size as f64),
        );
        maybe_gauge(&mut mf, &self.max_size, &l, Some(stats.max_size as f64));

        mf
    }
}
