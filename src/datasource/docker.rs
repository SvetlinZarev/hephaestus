use crate::metrics::docker::{ContainerStats, DataSource, DockerStats};
use anyhow::Context;
use std::collections::HashMap;

use bollard::models::{ContainerCpuStats, ContainerSummary};
use bollard::query_parameters::{ListContainersOptionsBuilder, StatsOptionsBuilder};
use futures::StreamExt;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::debug;

struct CpuStats {
    total: u64,
    system: u64,
}

pub struct DockerClient {
    prev_cpu_stats: Mutex<HashMap<String, CpuStats>>,
}

impl DockerClient {
    pub fn new() -> Self {
        Self {
            prev_cpu_stats: Mutex::new(HashMap::new()),
        }
    }
}

impl DataSource for DockerClient {
    async fn docker_stats(&self) -> anyhow::Result<DockerStats> {
        let docker = bollard::Docker::connect_with_unix_defaults()
            .context("Is the docker daemon running?")?;

        let stat_opts = Some(
            StatsOptionsBuilder::new()
                .stream(false)
                .one_shot(true)
                .build(),
        );
        let list_container_opts = Some(ListContainersOptionsBuilder::new().all(false).build());
        let containers = docker.list_containers(list_container_opts).await?;

        let mut container_stats = Vec::new();
        let mut current_cpu_stats = HashMap::new();
        let mut prev_cpu_stats = self.prev_cpu_stats.lock().await;

        for container in containers {
            let Some(id) = container.id.as_ref() else {
                debug!(container=?container, "Skipping container stats for container without ID");
                continue;
            };

            let name = container_name(&container);
            let mut stream = docker.stats(id, stat_opts.clone());

            if let Some(stats) = stream.next().await {
                let Ok(s) = &stats else {
                    debug!(?stats, "Skipping container stats because of an error");
                    continue;
                };

                let container_cpu_stats = s.cpu_stats.as_ref();
                let (cpu_usage, measurement) =
                    cpu_usage(&name, container_cpu_stats, &prev_cpu_stats);
                if let Some(measurement) = measurement {
                    current_cpu_stats.insert(name.clone(), measurement);
                }

                let mem_usage_bytes = s.memory_stats.as_ref().and_then(|m| m.usage);

                let (mut rx, mut tx) = (None, None);
                if let Some(net) = s.networks.as_ref() {
                    rx = Some(net.values().map(|n| n.rx_bytes.unwrap_or_default()).sum());
                    tx = Some(net.values().map(|n| n.tx_bytes.unwrap_or_default()).sum());
                }

                container_stats.push(ContainerStats {
                    name,
                    cpu_usage,
                    mem_usage_bytes,
                    net_rx_bytes: rx,
                    net_tx_bytes: tx,
                });
            }
        }

        *prev_cpu_stats = current_cpu_stats;
        Ok(DockerStats {
            timestamp: Instant::now(),
            containers: container_stats,
        })
    }
}

fn container_name(container: &ContainerSummary) -> String {
    container
        .names
        .as_ref()
        .and_then(|n| n.first().map(|s| s.as_str()))
        .unwrap_or_else(|| container.id.as_deref().unwrap_or("n/a"))
        .trim_start_matches('/')
        .to_string()
}

fn cpu_usage(
    container_name: &str,
    container_stats: Option<&ContainerCpuStats>,
    prev_measurements: &HashMap<String, CpuStats>,
) -> (Option<f64>, Option<CpuStats>) {
    let Some(container_stats) = container_stats else {
        return (None, None);
    };

    let Some(current) = to_cpu_stats(container_stats) else {
        return (None, None);
    };

    let Some(previous) = prev_measurements.get(container_name) else {
        return (None, Some(current));
    };

    if current.total <= previous.total || current.system <= previous.system {
        // most probably, the container has been restarted
        return (None, Some(current));
    }

    let cpus = container_stats.online_cpus.unwrap_or(1) as f64;
    let cpu_delta = (current.total - previous.total) as f64;
    let sys_delta = (current.system - previous.system) as f64;

    let mut usage = 0.0;
    if sys_delta > 0.0 && cpu_delta > 0.0 {
        usage = (cpu_delta / sys_delta) * cpus;
    }

    if usage > cpus {
        return (None, Some(current));
    }

    (Some(usage), Some(current))
}

fn to_cpu_stats(stats: &ContainerCpuStats) -> Option<CpuStats> {
    let total = total_cpu_usage(stats)?;
    let system = system_cpu_usage(stats)?;

    Some(CpuStats { total, system })
}

fn total_cpu_usage(stats: &ContainerCpuStats) -> Option<u64> {
    stats.cpu_usage.as_ref()?.total_usage
}

fn system_cpu_usage(stats: &ContainerCpuStats) -> Option<u64> {
    stats.system_cpu_usage
}
