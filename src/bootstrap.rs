use crate::config::Configuration;
use crate::datasource::TokioReader;
use crate::domain::{Collector, Metric};
use crate::{datasource, metrics};
use prometheus::Registry;

pub fn init_collectors(
    config: &Configuration,
    registry: &Registry,
) -> anyhow::Result<Vec<Box<dyn Collector>>> {
    let mut collectors = vec![];

    let mem_usage = metrics::memory_usage::MemoryUsage::new(config.collector.memory_usage.clone());
    let data_source = datasource::memory_usage::MemoryUsage::new(TokioReader::new());
    collectors.push(mem_usage.register(registry, data_source)?);

    let cpu_freq =
        metrics::cpu_frequency::CpuFrequency::new(config.collector.cpu_frequency.clone());
    let data_source = datasource::cpu_frequency::CpuFrequency::new(TokioReader::new());
    collectors.push(cpu_freq.register(registry, data_source)?);

    let cpu_usage = metrics::cpu_usage::CpuUsage::new(config.collector.cpu_usage.clone());
    let data_source = datasource::cpu_usage::CpuUsage::new(TokioReader::new());
    collectors.push(cpu_usage.register(registry, data_source)?);

    let net_io = metrics::network_io::NetworkIo::new(config.collector.network_io.clone());
    let data_source = datasource::network_io::NetworkIo::new(TokioReader::new());
    collectors.push(net_io.register(registry, data_source)?);

    let disk_io = metrics::disk_io::DiskIo::new(config.collector.disk_io.clone());
    let data_source = datasource::disk_io::DiskIo::new(TokioReader::new());
    collectors.push(disk_io.register(registry, data_source)?);
    
    let disk_temp = metrics::disk_smart::DiskTemp::new(config.collector.disk_temp.clone());
    let datasource = datasource::disk_smart::SmartCtl::new();
    collectors.push(disk_temp.register(registry, datasource)?);

    let ups = metrics::ups::Ups::new(config.collector.ups.clone());
    let data_source = datasource::nut::Nut::new(config.datasource.nut.clone());
    collectors.push(ups.register(registry, data_source)?);

    Ok(collectors)
}
