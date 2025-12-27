use crate::config::Collectors;
use crate::datasource::TokioReader;
use crate::domain::{Collector, Metric};
use crate::{datasource, metrics};
use prometheus::Registry;

pub fn init_collectors(
    config: &Collectors,
    registry: &Registry,
) -> anyhow::Result<Vec<Box<dyn Collector>>> {
    let mut collectors = vec![];

    let mem_usage = metrics::memory_usage::MemoryUsage::new(config.memory_usage.clone());
    let data_source = datasource::memory_usage::MemoryUsage::new(TokioReader::new());
    collectors.push(mem_usage.register(registry, data_source)?);

    Ok(collectors)
}
