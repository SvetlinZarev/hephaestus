use crate::metrics::docker::{ContainerStats, DataSource, DockerStats};
use anyhow::Context;
use std::collections::HashMap;

use bollard::models::{
    ContainerCpuStats, ContainerMemoryStats, ContainerNetworkStats, ContainerSummary,
};
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
    #[tracing::instrument(level = "debug", skip_all)]
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

                let mem_usage_bytes = calculate_memory_usage(s.memory_stats.as_ref());
                let (net_rx_bytes, net_tx_bytes) = calculate_network_usage(s.networks.as_ref());

                container_stats.push(ContainerStats {
                    name,
                    cpu_usage,
                    mem_usage_bytes,
                    net_rx_bytes,
                    net_tx_bytes,
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

fn calculate_memory_usage(stats: Option<&ContainerMemoryStats>) -> Option<u64> {
    let stats = stats?;
    let mem_stats = stats.stats.as_ref()?;

    let usage = stats.usage?;
    let inactive_file = mem_stats.get("inactive_file").copied().unwrap_or(0);

    Some(usage.saturating_sub(inactive_file))
}

fn calculate_network_usage(
    networks: Option<&HashMap<String, ContainerNetworkStats>>,
) -> (Option<u64>, Option<u64>) {
    let Some(net_map) = networks else {
        return (None, None);
    };

    let rx = net_map
        .values()
        .map(|n| n.rx_bytes.unwrap_or_default())
        .sum();

    let tx = net_map
        .values()
        .map(|n| n.tx_bytes.unwrap_or_default())
        .sum();

    (Some(rx), Some(tx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{
        ContainerCpuStats, ContainerCpuUsage, ContainerMemoryStats, ContainerNetworkStats,
        ContainerSummary,
    };
    use std::collections::HashMap;

    fn make_cpu_stats(total: u64, system: u64, cpus: u32) -> ContainerCpuStats {
        ContainerCpuStats {
            cpu_usage: Some(ContainerCpuUsage {
                total_usage: Some(total),
                ..Default::default()
            }),
            system_cpu_usage: Some(system),
            online_cpus: Some(cpus),
            ..Default::default()
        }
    }

    #[test]
    fn test_cpu_usage_no_previous_measurement() {
        let stats = make_cpu_stats(1_000_000, 5_000_000, 4);
        let prev = HashMap::new();

        let (usage, measurement) = cpu_usage("test", Some(&stats), &prev);
        assert!(usage.is_none());
        assert!(measurement.is_some());
        let m = measurement.unwrap();
        assert_eq!(m.total, 1_000_000);
        assert_eq!(m.system, 5_000_000);
    }

    #[test]
    fn test_cpu_usage_normal_calculation() {
        let stats = make_cpu_stats(2_000_000, 10_000_000, 4);
        let mut prev = HashMap::new();
        prev.insert(
            "test".to_string(),
            CpuStats {
                total: 1_000_000,
                system: 5_000_000,
            },
        );

        let (usage, measurement) = cpu_usage("test", Some(&stats), &prev);
        // cpu_delta = 1_000_000, sys_delta = 5_000_000
        // usage = (1_000_000 / 5_000_000) * 4 = 0.8
        assert!(usage.is_some());
        assert!((usage.unwrap() - 0.8).abs() < f64::EPSILON);
        assert!(measurement.is_some());
    }

    #[test]
    fn test_cpu_usage_container_restart() {
        // current.total <= previous.total indicates restart
        let stats = make_cpu_stats(500, 10_000_000, 4);
        let mut prev = HashMap::new();
        prev.insert(
            "test".to_string(),
            CpuStats {
                total: 1_000_000,
                system: 5_000_000,
            },
        );

        let (usage, measurement) = cpu_usage("test", Some(&stats), &prev);
        assert!(usage.is_none());
        assert!(measurement.is_some());
    }

    #[test]
    fn test_cpu_usage_exceeding_cpu_count() {
        let stats = make_cpu_stats(10_000_000, 2_000_000, 1);
        let mut prev = HashMap::new();
        prev.insert(
            "test".to_string(),
            CpuStats {
                total: 1_000_000,
                system: 1_000_000,
            },
        );

        let (usage, measurement) = cpu_usage("test", Some(&stats), &prev);
        assert!(usage.is_none());
        assert!(measurement.is_some());
    }

    #[test]
    fn test_cpu_usage_none_stats() {
        let prev = HashMap::new();
        let (usage, measurement) = cpu_usage("test", None, &prev);
        assert!(usage.is_none());
        assert!(measurement.is_none());
    }

    #[test]
    fn test_cpu_usage_zero_deltas() {
        let stats = make_cpu_stats(1_000_000, 5_000_000, 4);
        let mut prev = HashMap::new();
        prev.insert(
            "test".to_string(),
            CpuStats {
                total: 1_000_000,
                system: 5_000_000,
            },
        );

        let (usage, measurement) = cpu_usage("test", Some(&stats), &prev);
        // current.total == previous.total triggers restart detection
        assert!(usage.is_none());
        assert!(measurement.is_some());
    }

    #[test]
    fn test_memory_usage_normal() {
        let stats = ContainerMemoryStats {
            usage: Some(512_000_000),
            stats: Some(HashMap::from([("inactive_file".to_string(), 100_000_000)])),
            ..Default::default()
        };
        assert_eq!(calculate_memory_usage(Some(&stats)), Some(412_000_000));
    }

    #[test]
    fn test_memory_usage_no_inactive_file() {
        let stats = ContainerMemoryStats {
            usage: Some(512_000_000),
            stats: Some(HashMap::new()),
            ..Default::default()
        };
        assert_eq!(calculate_memory_usage(Some(&stats)), Some(512_000_000));
    }

    #[test]
    fn test_memory_usage_none_stats() {
        assert_eq!(calculate_memory_usage(None), None);
    }

    #[test]
    fn test_memory_usage_missing_usage_field() {
        let stats = ContainerMemoryStats {
            usage: None,
            stats: Some(HashMap::from([("inactive_file".to_string(), 100)])),
            ..Default::default()
        };
        assert_eq!(calculate_memory_usage(Some(&stats)), None);
    }

    #[test]
    fn test_memory_usage_missing_stats_map() {
        let stats = ContainerMemoryStats {
            usage: Some(512_000_000),
            stats: None,
            ..Default::default()
        };
        assert_eq!(calculate_memory_usage(Some(&stats)), None);
    }

    #[test]
    fn test_network_usage_multiple_interfaces() {
        let networks = HashMap::from([
            (
                "eth0".to_string(),
                ContainerNetworkStats {
                    rx_bytes: Some(1_000),
                    tx_bytes: Some(2_000),
                    ..Default::default()
                },
            ),
            (
                "eth1".to_string(),
                ContainerNetworkStats {
                    rx_bytes: Some(3_000),
                    tx_bytes: Some(4_000),
                    ..Default::default()
                },
            ),
        ]);

        let (rx, tx) = calculate_network_usage(Some(&networks));
        assert_eq!(rx, Some(4_000));
        assert_eq!(tx, Some(6_000));
    }

    #[test]
    fn test_network_usage_none() {
        let (rx, tx) = calculate_network_usage(None);
        assert!(rx.is_none());
        assert!(tx.is_none());
    }

    #[test]
    fn test_network_usage_empty_map() {
        let networks = HashMap::new();
        let (rx, tx) = calculate_network_usage(Some(&networks));
        assert_eq!(rx, Some(0));
        assert_eq!(tx, Some(0));
    }

    #[test]
    fn test_container_name_strips_leading_slash() {
        let container = ContainerSummary {
            names: Some(vec!["/my-container".to_string()]),
            ..Default::default()
        };
        assert_eq!(container_name(&container), "my-container");
    }

    #[test]
    fn test_container_name_falls_back_to_id() {
        let container = ContainerSummary {
            names: None,
            id: Some("abc123def456".to_string()),
            ..Default::default()
        };
        assert_eq!(container_name(&container), "abc123def456");
    }

    #[test]
    fn test_container_name_falls_back_to_na() {
        let container = ContainerSummary {
            names: None,
            id: None,
            ..Default::default()
        };
        assert_eq!(container_name(&container), "n/a");
    }
}
