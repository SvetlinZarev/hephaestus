use crate::datasource::Reader;
use crate::metrics::cpu_usage::{CoreStats, CoreUsageStats, CpuUsageStats, DataSource};
use std::sync::Mutex;
use tokio::time::{Duration, Instant};

const PATH_PROC_STAT: &str = "/proc/stat";
const MIN_TIME_BETWEEN_MEASUREMENTS: Duration = Duration::from_millis(250);

const CPU_USER: usize = 0;
const CPU_NICE: usize = 1;
const CPU_SYSTEM: usize = 2;
const CPU_IDLE: usize = 3;
const CPU_IOWAIT: usize = 4;
const CPU_IRQ: usize = 5;
const CPU_SOFTIRQ: usize = 6;
const CPU_STEAL: usize = 7;
const CPU_GUEST: usize = 8;
const CPU_GUEST_NICE: usize = 9;
const CPU_STATS_COUNT: usize = 10;

// We sum the first 8 columns for the total (User through Steal) because
// the kernel includes Guest and Guest_Nice inside User and Nice counters.
const CPU_TOTAL_COLUMNS: usize = 8;

pub struct CpuUsage<R> {
    reader: R,
    // [user, nice, system, idle, iowait, irq, softirq, steal, guest, guest_nice]
    measurement: Mutex<Option<(Instant, Vec<[u64; 10]>)>>,
}

impl<R> CpuUsage<R>
where
    R: Reader,
{
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            measurement: Mutex::new(None),
        }
    }
}

impl<R> DataSource for CpuUsage<R>
where
    R: Reader,
{
    #[allow(clippy::manual_async_fn)]
    fn cpu_usage(&self) -> impl Future<Output = anyhow::Result<CpuUsageStats>> + Send {
        async move {
            let Ok(mut previous) = self.measurement.lock().map(|x| x.clone()) else {
                return Err(anyhow::anyhow!(
                    "Failed to retrieve previous CPU usage measurement snapshot due to a poisoned lock"
                ));
            };

            let (timestamp, previous) = match previous.take() {
                Some(previous) => previous,
                None => {
                    let measurement = make_measurement(&self.reader).await?;
                    (Instant::now(), measurement)
                }
            };

            let time_since_measurement = timestamp.elapsed();
            if time_since_measurement < MIN_TIME_BETWEEN_MEASUREMENTS {
                let to_sleep = MIN_TIME_BETWEEN_MEASUREMENTS.saturating_sub(time_since_measurement);
                tokio::time::sleep(to_sleep).await;
            }

            let current = make_measurement(&self.reader).await?;
            let now = Instant::now();
            if previous.len() != current.len() {
                return Err(anyhow::anyhow!(
                    "Failed to perform CPU usage measurement because of changed core count: previous={}; current={}",
                    previous.len(),
                    current.len()
                ));
            }

            let (total_usage, total_breakdown) = calculate_usage(&current[0], &previous[0]);
            let cores = previous
                .iter()
                .zip(current.iter())
                .skip(1) // the first element is the "total" CPU usage across all cores
                .map(|(prev, curr)| calculate_usage(curr, prev))
                .enumerate()
                .map(|(core, (total_usage, breakdown))| CoreUsageStats {
                    core,
                    total_usage,
                    breakdown,
                })
                .collect::<Vec<_>>();

            match self.measurement.lock() {
                Ok(mut guard) => {
                    *guard = Some((now, current));
                }
                Err(_) => {
                    return Err(anyhow::anyhow!(
                        "Failed to update CPU usage measurement snapshot due to a poisoned lock"
                    ));
                }
            }

            Ok(CpuUsageStats {
                total_usage,
                total_breakdown,
                cores,
            })
        }
    }
}

async fn make_measurement<R: Reader>(reader: &R) -> anyhow::Result<Vec<[u64; 10]>> {
    let content = reader.read_to_string(PATH_PROC_STAT).await?;
    Ok(parse_proc_stat(&content))
}

fn parse_proc_stat(content: &str) -> Vec<[u64; 10]> {
    content
        .lines()
        .filter(|l| l.starts_with("cpu"))
        .map(|line| {
            let mut vals = [0u64; 10];
            for (idx, part) in line.split_whitespace().skip(1).take(10).enumerate() {
                vals[idx] = part.parse::<u64>().unwrap_or(0);
            }

            vals
        })
        .collect()
}

fn calculate_usage(curr: &[u64; 10], prev: &[u64; 10]) -> (f64, CoreStats) {
    let mut deltas = [0u64; CPU_STATS_COUNT];
    for i in 0..CPU_STATS_COUNT {
        deltas[i] = curr[i].saturating_sub(prev[i]);
    }

    // The kernel includes Guest and Guest_Nice inside User and Nice counters.
    // To calculate the actual total elapsed time, we sum columns 0 through 7.
    let total_delta: u64 = deltas.iter().take(CPU_TOTAL_COLUMNS).sum();
    if total_delta == 0 {
        return (0.0, CoreStats::default());
    }

    let t = total_delta as f64;

    let times = CoreStats {
        user: deltas[CPU_USER] as f64 / t,
        nice: deltas[CPU_NICE] as f64 / t,
        system: deltas[CPU_SYSTEM] as f64 / t,
        idle: deltas[CPU_IDLE] as f64 / t,
        iowait: deltas[CPU_IOWAIT] as f64 / t,
        irq: deltas[CPU_IRQ] as f64 / t,
        softirq: deltas[CPU_SOFTIRQ] as f64 / t,
        steal: deltas[CPU_STEAL] as f64 / t,
        guest: deltas[CPU_GUEST] as f64 / t,
        guest_nice: deltas[CPU_GUEST_NICE] as f64 / t,
    };

    let busy_percentage = 1.0 - times.idle - times.iowait;
    (busy_percentage.max(0.0), times)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasource::tests::HardcodedReader;
    use std::time::Duration;

    #[test]
    fn test_parse_proc_stat_valid_input() {
        // Raw string allows us to visualize the file structure exactly as it appears in /proc
        let content = r#"cpu  1100 200 300 400 500 600 700 800 900 1000
cpu0 600 100 150 200 250 300 350 400 450 500
cpu1 500 100 150 200 250 300 350 400 450 500
intr 123456 789
ctxt 987654
"#;

        let result = parse_proc_stat(content);
        assert_eq!(result.len(), 3);

        // Values: user nice system idle iowait irq softirq steal guest guest_nice
        assert_eq!(
            result[0],
            [1100, 200, 300, 400, 500, 600, 700, 800, 900, 1000]
        );
        assert_eq!(
            result[1],
            [600, 100, 150, 200, 250, 300, 350, 400, 450, 500]
        );
        assert_eq!(
            result[2],
            [500, 100, 150, 200, 250, 300, 350, 400, 450, 500]
        );
    }

    #[test]
    fn test_parse_proc_stat_with_varying_whitespace() {
        // Test that the parser handles the double-space after 'cpu'
        // and potential trailing spaces or empty lines.
        let content = r#"cpu  10 10 10 10 10 10 10 10 10 10 
cpu0 20 20 20 20 20 20 20 20 20 20

"#;

        let result = parse_proc_stat(content);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0][0], 10);
        assert_eq!(result[1][0], 20);
    }

    #[test]
    fn test_calculate_usage_standard_usage() {
        // prev: 100 total (all idle)
        let prev = [0, 0, 0, 100, 0, 0, 0, 0, 0, 0];

        // curr: 200 total (added: 20 user, 10 system, 10 iowait, 60 idle)
        let curr = [20, 0, 10, 160, 10, 0, 0, 0, 0, 0];

        let (usage, stats) = calculate_usage(&curr, &prev);

        // Total Delta = (20+0+10+60+10+0+0+0) = 100
        // usage = 1.0 - idle_ratio - iowait_ratio
        // usage = 1.0 - 0.6 - 0.1 = 0.3
        assert!((usage - 0.3).abs() < f64::EPSILON);

        // Individual Stats
        assert!((stats.user - 0.2).abs() < f64::EPSILON);
        assert!((stats.system - 0.1).abs() < f64::EPSILON);
        assert!((stats.idle - 0.6).abs() < f64::EPSILON);
        assert!((stats.iowait - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculate_usage_zero_delta() {
        let prev = [100, 100, 100, 100, 100, 100, 100, 100, 100, 100];
        let curr = [100, 100, 100, 100, 100, 100, 100, 100, 100, 100];

        let (usage, stats) = calculate_usage(&curr, &prev);

        assert_eq!(usage, 0.0);
        assert_eq!(stats.user, 0.0);
        assert_eq!(stats.idle, 0.0);
    }

    #[test]
    fn test_calculate_usage_guest_overlap() {
        // Guest (index 8) and GuestNice (index 9) are usually inside User/Nice.
        let prev = [0u64; 10];
        let curr = [
            10, // user (includes 5 guest)
            0,  // nice
            0,  // system
            90, // idle
            0,  // iowait
            0,  // irq
            0,  // softirq
            0,  // steal
            5,  // guest
            0,  // guest_nice
        ];

        let (usage, stats) = calculate_usage(&curr, &prev);

        assert!((stats.user - 0.1).abs() < f64::EPSILON);
        assert!((stats.guest - 0.05).abs() < f64::EPSILON);
        assert!((usage - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_calculate_usage_saturating_sub() {
        // Ensure that if counters roll over or are weirdly smaller, we don't panic
        let prev = [200; 10];
        let curr = [100; 10];

        let (usage, _) = calculate_usage(&curr, &prev);
        assert_eq!(usage, 0.0);
    }

    #[tokio::test]
    async fn test_cpu_usage_datasource() {
        // Snapshot A: Baseline (Total 100, Idle 100)
        let snapshot_a = r#"cpu  0 0 0 100 0 0 0 0 0 0
cpu0 0 0 0 100 0 0 0 0 0 0
"#;
        // Snapshot B: First delta (Added 100 jiffies, 50 user, 50 idle) -> 50% usage
        let snapshot_b = r#"cpu  50 0 0 150 0 0 0 0 0 0
cpu0 50 0 0 150 0 0 0 0 0 0
"#;
        // Snapshot C: Second delta (Added 100 jiffies, 80 system, 20 idle) -> 80% usage
        let snapshot_c = r#"cpu  50 0 80 170 0 0 0 0 0 0
cpu0 50 0 80 170 0 0 0 0 0 0
"#;

        let mut reader = HardcodedReader::new();
        reader.add_response(PATH_PROC_STAT, snapshot_a);
        reader.add_response(PATH_PROC_STAT, snapshot_b);
        reader.add_response(PATH_PROC_STAT, snapshot_c);

        let datasource = CpuUsage::new(reader);

        // Pause time to make the 250ms sleep instant
        tokio::time::pause();

        // Assertions for Snapshot A -> B
        let first_stats = datasource.cpu_usage().await.unwrap();
        assert!((first_stats.total_usage - 0.5).abs() < f64::EPSILON);
        assert!((first_stats.total_breakdown.user - 0.5).abs() < f64::EPSILON);
        assert_eq!(first_stats.cores.len(), 1);

        // Advance time so we don't trigger another sleep inside the method
        tokio::time::advance(Duration::from_millis(300)).await;

        // Assertions for Snapshot B -> C
        // Delta: user=0, system=80, idle=20. Total=100.
        // Usage: 1.0 - 0.2 = 0.8
        let second_stats = datasource.cpu_usage().await.unwrap();
        assert!((second_stats.total_usage - 0.8).abs() < f64::EPSILON);
        assert!((second_stats.total_breakdown.system - 0.8).abs() < f64::EPSILON);
        assert!((second_stats.total_breakdown.user - 0.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_cpu_usage_all_metrics_mapping() {
        // Snapshot A: All counters at 100
        let snapshot_a = r#"cpu  100 100 100 100 100 100 100 100 100 100
cpu0 100 100 100 100 100 100 100 100 100 100"#;

        // Snapshot B: Every column increased by a specific unique amount
        // Total Delta (first 8) = 10 + 20 + 30 + 40 + 50 + 60 + 70 + 80 = 360
        let snapshot_b = r#"cpu  110 120 130 140 150 160 170 180 190 200
cpu0 110 120 130 140 150 160 170 180 190 200"#;

        let mut reader = HardcodedReader::new();
        reader.add_response(PATH_PROC_STAT, snapshot_a);
        reader.add_response(PATH_PROC_STAT, snapshot_b);

        let datasource = CpuUsage::new(reader);
        tokio::time::pause();

        let stats = datasource.cpu_usage().await.unwrap();
        let t = 360.0; // The divisor (sum of first 8 deltas)
        let c0 = &stats.cores[0].breakdown;

        // Column 0: User (Delta 10)
        assert!(
            (c0.user - (10.0 / t)).abs() < f64::EPSILON,
            "User ratio fail"
        );

        // Column 1: Nice (Delta 20)
        assert!(
            (c0.nice - (20.0 / t)).abs() < f64::EPSILON,
            "Nice ratio fail"
        );

        // Column 2: System (Delta 30)
        assert!(
            (c0.system - (30.0 / t)).abs() < f64::EPSILON,
            "System ratio fail"
        );

        // Column 3: Idle (Delta 40)
        assert!(
            (c0.idle - (40.0 / t)).abs() < f64::EPSILON,
            "Idle ratio fail"
        );

        // Column 4: IOWait (Delta 50)
        assert!(
            (c0.iowait - (50.0 / t)).abs() < f64::EPSILON,
            "IOWait ratio fail"
        );

        // Column 5: IRQ (Delta 60)
        assert!((c0.irq - (60.0 / t)).abs() < f64::EPSILON, "IRQ ratio fail");

        // Column 6: SoftIRQ (Delta 70)
        assert!(
            (c0.softirq - (70.0 / t)).abs() < f64::EPSILON,
            "SoftIRQ ratio fail"
        );

        // Column 7: Steal (Delta 80)
        assert!(
            (c0.steal - (80.0 / t)).abs() < f64::EPSILON,
            "Steal ratio fail"
        );

        // Column 8: Guest (Delta 90) - Note: divisor is still 360
        assert!(
            (c0.guest - (90.0 / t)).abs() < f64::EPSILON,
            "Guest ratio fail"
        );

        // Column 9: Guest Nice (Delta 100)
        assert!(
            (c0.guest_nice - (100.0 / t)).abs() < f64::EPSILON,
            "GuestNice ratio fail"
        );

        // Usage = 1.0 - idle - iowait
        // Usage = 1.0 - (40/360) - (50/360) = 1.0 - (90/360) = 1.0 - 0.25 = 0.75
        assert!(
            (stats.cores[0].total_usage - 0.75).abs() < f64::EPSILON,
            "Total Usage calculation fail"
        );
    }
}
