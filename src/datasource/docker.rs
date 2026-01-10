use crate::metrics::docker::{ContainerStats, DataSource, DockerStats};
use bollard::query_parameters::{ListContainersOptionsBuilder, StatsOptions};
use bollard::Docker;
use futures::StreamExt;
use tokio::time::Instant;

pub struct DockerClient {
    docker: Docker,
}

impl DockerClient {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            docker: Docker::connect_with_local_defaults()?,
        })
    }
}

impl DataSource for DockerClient {
    async fn docker_stats(&self) -> anyhow::Result<DockerStats> {
        let params = ListContainersOptionsBuilder::new().all(false).build();
        let containers = self.docker.list_containers(Some(params)).await?;

        let mut container_stats = Vec::new();
        for container in containers {
            let id = container.id.unwrap_or_default();
            let name = container
                .names
                .as_ref()
                .and_then(|n| n.first())
                .cloned()
                .unwrap_or_else(|| id.clone())
                .trim_start_matches('/')
                .to_string();

            let mut stream = self.docker.stats(
                &id,
                Some(StatsOptions {
                    stream: false,
                    one_shot: true,
                }),
            );

            if let Some(Ok(s)) = stream.next().await {
                let cpu_stats = s.cpu_stats.unwrap_or_default();
                let precpu_stats = s.precpu_stats.unwrap_or_default();

                // Fix E0609: Handle nested Option<ContainerCpuUsage>
                let current_usage = cpu_stats
                    .cpu_usage
                    .map(|u| u.total_usage)
                    .flatten()
                    .unwrap_or_default() as f64;

                let pre_usage = precpu_stats
                    .cpu_usage
                    .map(|u| u.total_usage)
                    .flatten()
                    .unwrap_or_default() as f64;

                let cpu_delta = current_usage - pre_usage;

                let sys_delta = cpu_stats.system_cpu_usage.unwrap_or_default() as f64
                    - precpu_stats.system_cpu_usage.unwrap_or_default() as f64;

                let cpus = cpu_stats.online_cpus.unwrap_or(1) as f64;
                let cpu_pct = if sys_delta > 0.0 {
                    (cpu_delta / sys_delta) * cpus * 100.0
                } else {
                    0.0
                };

                let net = s.networks.unwrap_or_default();
                let rx = net.values().map(|n| n.rx_bytes.unwrap_or_default()).sum();
                let tx = net.values().map(|n| n.tx_bytes.unwrap_or_default()).sum();

                container_stats.push(ContainerStats {
                    name,
                    cpu_usage_pct: cpu_pct,
                    mem_usage_bytes: s.memory_stats.and_then(|m| m.usage).unwrap_or_default()
                        as f64,
                    net_rx_bytes: rx,
                    net_tx_bytes: tx,
                });
            }
        }

        Ok(DockerStats {
            timestamp: Instant::now(),
            containers: container_stats,
        })
    }
}
