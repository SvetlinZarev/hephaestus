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

    let cpu_freq = metrics::cpu_frequency::CpuFrequency::new(config.cpu_frequency.clone());
    let data_source = datasource::cpu_frequency::CpuFrequency::new(TokioReader::new());
    collectors.push(cpu_freq.register(registry, data_source)?);

    let net_io = metrics::network_io::NetworkIo::new(config.network_io.clone());
    let data_source = datasource::network_io::NetworkIo::new(TokioReader::new());
    collectors.push(net_io.register(registry, data_source)?);

    let disk_io = metrics::disk_io::DiskIo::new(config.disk_io.clone());
    let data_source = datasource::disk_io::DiskIo::new(TokioReader::new());
    collectors.push(disk_io.register(registry, data_source)?);

    Ok(collectors)
}
