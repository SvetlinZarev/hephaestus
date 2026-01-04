use crate::datasource::Reader;
use crate::metrics::zfs_dataset::{DataSource, DatasetIoStats, ZfsIoStats};
use tokio::fs;

const KSTAT_ZFS: &str = "/proc/spl/kstat/zfs";

pub struct KstatZfsDatasetIo<R> {
    reader: R,
}

impl<R> KstatZfsDatasetIo<R>
where
    R: Reader,
{
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    fn parse_objset(&self, pool: &str, content: &str) -> Option<DatasetIoStats> {
        let mut ds_name = String::new();
        let (mut reads, mut writes, mut nread, mut nwritten) = (0, 0, 0, 0);

        // ZFS kstats have 2 header lines
        for line in content.lines().skip(2) {
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            parts.next(); // skip type field
            let value = parts.next()?;

            match name {
                "dataset_name" => ds_name = value.to_string(),
                "reads" => reads = value.parse().unwrap_or(0),
                "writes" => writes = value.parse().unwrap_or(0),
                "nread" => nread = value.parse().unwrap_or(0),
                "nwritten" => nwritten = value.parse().unwrap_or(0),
                _ => {}
            }
        }

        // Filter out snapshots and empty entries
        if ds_name.is_empty() || ds_name.contains('@') {
            return None;
        }

        Some(DatasetIoStats {
            pool: pool.to_string(),
            dataset: ds_name,
            reads,
            writes,
            nread,
            nwritten,
        })
    }
}

impl<R> DataSource for KstatZfsDatasetIo<R>
where
    R: Reader,
{
    async fn dataset_io(&self) -> anyhow::Result<ZfsIoStats> {
        let mut datasets = Vec::new();

        let mut pool_entries = fs::read_dir(KSTAT_ZFS).await?;
        while let Some(pool_entry) = pool_entries.next_entry().await? {
            let path = pool_entry.path();
            if !path.is_dir() {
                continue;
            }

            let pool_name = pool_entry.file_name();
            let pool_name = pool_name.to_string_lossy();

            let mut objset_entries = fs::read_dir(&path).await?;
            while let Some(obj_entry) = objset_entries.next_entry().await? {
                let filename = obj_entry.file_name();
                let filename = filename.to_string_lossy();
                if !filename.starts_with("objset-") {
                    continue;
                }

                let content = self.reader.read_to_string(obj_entry.path()).await?;
                if let Some(ds_stats) = self.parse_objset(&pool_name, &content) {
                    datasets.push(ds_stats);
                }
            }
        }

        Ok(ZfsIoStats {
            timestamp: tokio::time::Instant::now(),
            datasets,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasource::tests::HardcodedReader;
    use std::io::ErrorKind;

    fn mock_objset_body(name: &str, reads: u64, writes: u64) -> String {
        format!(
            "250 1 0x01 1 1\n\
             name type value\n\
             dataset_name string {name}\n\
             reads u64 {reads}\n\
             writes u64 {writes}\n\
             nread u64 {nread}\n\
             nwritten u64 {nwritten}",
            nread = reads * 1024,
            nwritten = writes * 1024
        )
    }

    #[tokio::test]
    async fn test_parse_objset_logic() {
        let reader = HardcodedReader::new();
        let ds = KstatZfsDatasetIo::new(reader);

        // 1. Test valid filesystem
        let content = mock_objset_body("tank/home", 100, 50);
        let stats = ds.parse_objset("tank", &content).unwrap();
        assert_eq!(stats.dataset, "tank/home");
        assert_eq!(stats.reads, 100);
        assert_eq!(stats.writes, 50);

        // 2. Test snapshot filtering (should return None)
        let snap_content = mock_objset_body("tank/home@snap1", 10, 10);
        let stats = ds.parse_objset("tank", &snap_content);
        assert!(stats.is_none(), "Snapshots must be filtered out");

        // 3. Test malformed content (missing dataset_name)
        let malformed = "header\nheader\nreads u64 100";
        let stats = ds.parse_objset("tank", malformed);
        assert!(stats.is_none());
    }

    #[tokio::test]
    async fn test_dataset_io_sequential_updates() -> anyhow::Result<()> {
        let mut reader = HardcodedReader::new();
        let path = "/proc/spl/kstat/zfs/tank/objset-0x1";

        // Scenario: Dataset IO increases over two scrapes
        reader.add_response(path, mock_objset_body("tank/data", 100, 50));
        reader.add_response(path, mock_objset_body("tank/data", 110, 55));
        let ds = KstatZfsDatasetIo::new(reader);

        let content1 = ds.reader.read_to_string(path).await?;
        let stats1 = ds.parse_objset("tank", &content1).unwrap();
        assert_eq!(stats1.reads, 100);

        let content2 = ds.reader.read_to_string(path).await?;
        let stats2 = ds.parse_objset("tank", &content2).unwrap();
        assert_eq!(stats2.reads, 110);

        Ok(())
    }

    #[tokio::test]
    async fn test_dataset_io_not_found() {
        let reader = HardcodedReader::new(); // No paths added
        let ds = KstatZfsDatasetIo::new(reader);

        let result = ds.reader.read_to_string("/non/existent/path").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_parsing_with_other_kstats_present() {
        let reader = HardcodedReader::new();
        let ds = KstatZfsDatasetIo::new(reader);

        // Ensure that extra kstat fields don't break the parser
        let content = format!(
            "{}\nrandom_stat u64 12345\nanother_one string hello",
            mock_objset_body("tank/test", 5, 5)
        );

        let stats = ds.parse_objset("tank", &content).unwrap();
        assert_eq!(stats.dataset, "tank/test");
        assert_eq!(stats.reads, 5);
    }
}
