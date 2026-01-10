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

    let data_source = datasource::memory_usage::MemoryUsage::new(TokioReader::new());
    let mem_usage =
        metrics::memory_usage::MemoryUsage::new(config.collector.memory_usage.clone(), data_source);
    collectors.push(mem_usage.register(registry)?);

    let data_source = datasource::cpu_frequency::CpuFrequency::new(TokioReader::new());
    let cpu_freq = metrics::cpu_frequency::CpuFrequency::new(
        config.collector.cpu_frequency.clone(),
        data_source,
    );
    collectors.push(cpu_freq.register(registry)?);

    let data_source = datasource::cpu_usage::CpuUsage::new(TokioReader::new());
    let cpu_usage =
        metrics::cpu_usage::CpuUsage::new(config.collector.cpu_usage.clone(), data_source);
    collectors.push(cpu_usage.register(registry)?);

    let data_source = datasource::network_io::NetworkIo::new(TokioReader::new());
    let net_io =
        metrics::network_io::NetworkIo::new(config.collector.network_io.clone(), data_source);
    collectors.push(net_io.register(registry)?);

    let data_source = datasource::disk_io::DiskIo::new(TokioReader::new());
    let disk_io = metrics::disk_io::DiskIo::new(config.collector.disk_io.clone(), data_source);
    collectors.push(disk_io.register(registry)?);

    let data_source = datasource::disk_smart::SmartCtl::new();
    let disk_temp =
        metrics::disk_smart::Smart::new(config.collector.disk_temp.clone(), data_source);
    collectors.push(disk_temp.register(registry)?);

    let data_source = datasource::nut::Nut::new(config.datasource.nut.clone());
    let ups = metrics::ups::Ups::new(config.collector.ups.clone(), data_source);
    collectors.push(ups.register(registry)?);

    let data_source = datasource::zfs_arc::KstatZfs::new(TokioReader::new());
    let zfs_arc = metrics::zfs_arc::ZfsArc::new(config.collector.zfs_arc.clone(), data_source);
    collectors.push(zfs_arc.register(registry)?);

    let data_source = datasource::zfs_dataset::KstatZfsDatasetIo::new(TokioReader::new());
    let zfs_dataset =
        metrics::zfs_dataset::ZfsDatasetIo::new(config.collector.zfs_dataset.clone(), data_source);
    collectors.push(zfs_dataset.register(registry)?);

    let data_source = datasource::docker::DockerClient::new()?;
    let docker = metrics::docker::Docker::new(config.collector.docker.clone(), data_source);
    collectors.push(docker.register(registry)?);

    Ok(collectors)
}
