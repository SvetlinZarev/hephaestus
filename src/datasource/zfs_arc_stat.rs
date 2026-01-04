use crate::datasource::Reader;
use crate::metrics::zfs_arc_stat::{ArcStats, DataSource};
use tokio::time::Instant;

const PATH_ARCSTATS: &str = "/proc/spl/kstat/zfs/arcstats";

pub struct KstatZfs<R> {
    reader: R,
}

impl<R> KstatZfs<R>
where
    R: Reader,
{
    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R> DataSource for KstatZfs<R>
where
    R: Reader,
{
    async fn arc_stats(&self) -> anyhow::Result<ArcStats> {
        let content = self.reader.read_to_string(PATH_ARCSTATS).await?;
        let timestamp = Instant::now();
        let mut stats = ArcStats {
            timestamp,
            hits: 0,
            misses: 0,
            size: 0,
            target_size: 0,
            max_size: 0,
        };

        // arcstats format:
        // line 0: header
        // line 1: header
        // line 2+: name type value
        for line in content.lines().skip(2) {
            let mut parts = line.split_whitespace();
            let Some(name) = parts.next() else { continue };

            parts.next(); // skip type
            let Some(value_str) = parts.next() else {
                continue;
            };

            let Ok(value) = value_str.parse::<u64>() else {
                continue;
            };

            match name {
                "hits" => stats.hits = value,
                "misses" => stats.misses = value,
                "size" => stats.size = value,
                "c" => stats.target_size = value,
                "c_max" => stats.max_size = value,
                _ => {}
            }
        }

        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use crate::datasource::tests::HardcodedReader;
    use crate::datasource::zfs_arc_stat::{KstatZfs, PATH_ARCSTATS};
    use crate::metrics::zfs_arc_stat::DataSource;

    fn mock_arcstats_body(hits: u64, misses: u64, size: u64) -> String {
        format!(
            "250 1 0x01 1 1\n\
             name type value\n\
             hits 4 {hits}\n\
             misses 4 {misses}\n\
             size 4 {size}\n\
             c 4 2000\n\
             c_max 4 4000"
        )
    }

    #[tokio::test]
    async fn test_arc_stats_parsing_success() -> anyhow::Result<()> {
        let mut reader = HardcodedReader::new();
        let content = mock_arcstats_body(500, 100, 1024);
        reader.add_response(PATH_ARCSTATS, content);

        let data_source = KstatZfs::new(reader);
        let stats = data_source.arc_stats().await?;

        assert_eq!(stats.hits, 500);
        assert_eq!(stats.misses, 100);
        assert_eq!(stats.size, 1024);
        assert_eq!(stats.target_size, 2000);
        assert_eq!(stats.max_size, 4000);

        Ok(())
    }

    #[tokio::test]
    async fn test_arc_stats_sequential_reads() -> anyhow::Result<()> {
        let mut reader = HardcodedReader::new();
        reader.add_response(PATH_ARCSTATS, mock_arcstats_body(10, 5, 100));
        reader.add_response(PATH_ARCSTATS, mock_arcstats_body(20, 10, 200));

        let data_source = KstatZfs::new(reader);

        let stats1 = data_source.arc_stats().await?;
        assert_eq!(stats1.hits, 10);

        let stats2 = data_source.arc_stats().await?;
        assert_eq!(stats2.hits, 20);

        Ok(())
    }

    #[tokio::test]
    async fn test_arc_stats_missing_file() {
        let reader = HardcodedReader::new(); // No response added
        let data_source = KstatZfs::new(reader);

        let result = data_source.arc_stats().await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("File not found"));
    }

    #[tokio::test]
    async fn test_arc_stats_malformed_values() -> anyhow::Result<()> {
        let mut reader = HardcodedReader::new();
        let malformed = "header\nheader\nhits 4 NOT_A_NUMBER\nmisses 4 50";
        reader.add_response(PATH_ARCSTATS, malformed);

        let data_source = KstatZfs::new(reader);
        let stats = data_source.arc_stats().await?;

        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 50);
        Ok(())
    }
}
