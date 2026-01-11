use crate::datasource::Reader;
use crate::metrics::memory_usage::{DataSource, RamStats, SwapStats};

const PATH_MEM_INFO: &str = "/proc/meminfo";

pub struct MemoryUsage<R> {
    reader: R,
}

impl<R> MemoryUsage<R>
where
    R: Reader,
{
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    fn parse_line<'l>(&self, line: &'l str) -> Option<(&'l str, u64)> {
        let Some((key, rest)) = line.split_once(':') else {
            tracing::info!("Skipping invalid mem-info line: {}", line);
            return None;
        };

        let (value, unit) = match rest.trim_start().rsplit_once(' ') {
            None => (rest, ""),
            Some((value, unit)) => (value, unit),
        };

        let value = value.trim().parse::<u64>().unwrap_or(0);
        let unit = unit.trim();

        let value = match unit {
            "" => value,
            "kB" => value * 1024,
            "mB" => value * 1024 * 1024,
            _ => {
                tracing::info!(
                    "unit" = unit,
                    "Invalid unit found in mem-info line: {}",
                    line
                );
                return None;
            }
        };

        Some((key, value))
    }
}

impl<R> DataSource for MemoryUsage<R>
where
    R: Reader,
{
    #[tracing::instrument(level = "debug", skip_all)]
    async fn swap(&self) -> anyhow::Result<SwapStats> {
        let mut total = 0;
        let mut free = 0;

        let mem_info = self.reader.read_to_string(PATH_MEM_INFO).await?;
        for line in mem_info.lines() {
            let Some((key, value)) = self.parse_line(line) else {
                continue;
            };

            match key {
                "SwapTotal" => total = value,
                "SwapFree" => free = value,
                _ => {}
            }
        }

        let used = total.saturating_sub(free);
        Ok(SwapStats { total, used, free })
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn ram(&self) -> anyhow::Result<RamStats> {
        let mut total = 0;
        let mut free = 0;
        let mut available = 0;
        let mut buffers = 0;
        let mut cached = 0;
        let mut sreclaimable = 0;

        let mem_info = self.reader.read_to_string(PATH_MEM_INFO).await?;
        for line in mem_info.lines() {
            let Some((key, value)) = self.parse_line(line) else {
                continue;
            };

            match key {
                "MemTotal" => total = value,
                "MemFree" => free = value,
                "MemAvailable" => available = value,
                "Buffers" => buffers = value,
                "Cached" => cached = value,
                "SReclaimable" => sreclaimable = value,
                _ => {}
            }
        }

        let cache_total = cached + sreclaimable;
        let used = total
            .saturating_sub(free)
            .saturating_sub(buffers)
            .saturating_sub(cache_total);

        Ok(RamStats {
            total,
            used,
            free,
            available,
            buffers,
            cache: cache_total,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::datasource::memory_usage::{MemoryUsage, PATH_MEM_INFO};
    use crate::datasource::tests::HardcodedReader;
    use crate::metrics::memory_usage::DataSource;

    const MEM_INFO: &'static str = r#"MemTotal:       61489320 kB
MemFree:        44422752 kB
MemAvailable:   54097832 kB
Buffers:            1112 kB
Cached:          9113108 kB
SwapCached:            0 kB
Active:          8537148 kB
Inactive:        7160620 kB
Active(anon):    5594376 kB
Inactive(anon):        0 kB
Active(file):    2942772 kB
Inactive(file):  7160620 kB
Unevictable:        5764 kB
Mlocked:            5764 kB
SwapTotal:       8388604 kB
SwapFree:        2097152 kB
Zswap:                 0 kB
Zswapped:              0 kB
Dirty:              2004 kB
Writeback:             0 kB
AnonPages:       6438384 kB
Mapped:          1520572 kB
Shmem:             58640 kB
KReclaimable:     266068 kB
Slab:             539688 kB
SReclaimable:     266068 kB
SUnreclaim:       273620 kB
KernelStack:       26048 kB
PageTables:        57932 kB
SecPageTables:      4252 kB
NFS_Unstable:          0 kB
Bounce:                0 kB
WritebackTmp:          0 kB
CommitLimit:    30744660 kB
Committed_AS:   14825576 kB
VmallocTotal:   34359738367 kB
VmallocUsed:      185488 kB
VmallocChunk:          0 kB
Percpu:            19328 kB
HardwareCorrupted:     0 kB
AnonHugePages:   3209216 kB
ShmemHugePages:        0 kB
ShmemPmdMapped:        0 kB
FileHugePages:    102400 kB
FilePmdMapped:         0 kB
Unaccepted:            0 kB
Balloon:               0 kB
HugePages_Total:       0
HugePages_Free:        0
HugePages_Rsvd:        0
HugePages_Surp:        0
Hugepagesize:       2048 kB
Hugetlb:               0 kB
DirectMap4k:      537384 kB
DirectMap2M:    13916160 kB
DirectMap1G:    49283072 kB
"#;

    #[tokio::test]
    async fn test_parse_ram_meminfo() {
        let mut reader = HardcodedReader::new();
        reader.add_response(PATH_MEM_INFO, MEM_INFO);

        let ds = MemoryUsage::new(reader);
        let ram = ds.ram().await.expect("Failed to read RAM usage statistics");
        assert_eq!(ram.total, 62_965_063_680);
        assert_eq!(ram.free, 45_488_898_048);
        assert_eq!(ram.available, 55_396_179_968);
        assert_eq!(ram.cache, 9_604_276_224);
        assert_eq!(ram.buffers, 1_138_688);
        assert_eq!(ram.used, 7_870_750_720);
    }

    #[tokio::test]
    async fn test_parse_swap_meminfo() {
        let mut reader = HardcodedReader::new();
        reader.add_response(PATH_MEM_INFO, MEM_INFO);

        let ds = MemoryUsage::new(reader);
        let swap = ds
            .swap()
            .await
            .expect("Failed to read SWAP usage statistics");
        assert_eq!(swap.total, 8_589_930_496);
        assert_eq!(swap.free, 2_147_483_648);
        assert_eq!(swap.used, 6_442_446_848);
    }
}
