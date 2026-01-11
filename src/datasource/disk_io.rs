use crate::datasource::Reader;
use crate::metrics::disk_io::{DataSource, DeviceIoStats, DiskIoStats};
use tokio::time::Instant;

const PATH_DISK_STATS: &str = "/proc/diskstats";
const KERNEL_SECTOR_SIZE: u64 = 512;

pub struct DiskIo<R> {
    reader: R,
}

impl<R> DiskIo<R>
where
    R: Reader,
{
    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R> DataSource for DiskIo<R>
where
    R: Reader,
{
    #[tracing::instrument(level = "debug", skip_all)]
    async fn disk_io(&self) -> anyhow::Result<DiskIoStats> {
        let content = self.reader.read_to_string(PATH_DISK_STATS).await?;
        let timestamp = Instant::now();

        let mut disks = Vec::new();
        for line in content.lines() {
            let mut parts = line.split_whitespace();

            // Skip major and minor numbers (Columns 0 and 1)
            parts.next();
            parts.next();

            // Column 2: Device Name
            let Some(device) = parts.next() else {
                tracing::debug!("Missing device name: {}", line);
                continue;
            };

            if device.starts_with("loop")
                || device.starts_with("zram")
                || device.starts_with("md1p")
            {
                tracing::debug!("Skipping device: {}", device);
                continue;
            }

            // Column 3: Reads Completed
            let read_ops = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);

            // Column 4: Reads Merged (skip)
            parts.next();

            // Column 5: Sectors Read -> Bytes
            let sectors_read = parts
                .next()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let bytes_read = sectors_read * KERNEL_SECTOR_SIZE;

            // Column 6: Time spent reading (skip)
            parts.next();

            // Column 7: Writes Completed
            let write_ops = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);

            // Column 8: Writes Merged (skip)
            parts.next();

            // Column 9: Sectors Written -> Bytes
            let sectors_written = parts
                .next()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let bytes_written = sectors_written * KERNEL_SECTOR_SIZE;

            disks.push(DeviceIoStats {
                device_name: device.to_string(),
                bytes_read,
                bytes_written,
                read_ops,
                write_ops,
            });
        }

        Ok(DiskIoStats { timestamp, disks })
    }
}

#[cfg(test)]
mod tests {
    use crate::datasource::disk_io::{DiskIo, PATH_DISK_STATS};
    use crate::datasource::tests::HardcodedReader;
    use crate::metrics::disk_io::DataSource;

    const DISK_STATS: &str = r#"   7       0 loop0 133549 0 8587416 51112 0 0 0 0 0 13992709 51112 0 0 0 0 0 0
   7       1 loop1 599 0 100360 25 0 0 0 0 0 31 25 0 0 0 0 0 0
   7       2 loop2 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0
   7       3 loop3 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0
 252       0 zram0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0
 259       4 nvme0n1 2745204 0 554989650 559677 1793083 0 67354640 334639 0 584873 1134742 59324 0 7646410160 198553 158575 41872
 259       5 nvme0n1p1 2745099 0 554986698 559672 1793083 0 67354640 334639 0 967799 1092866 59324 0 7646410160 198553 0 0
   8       0 sda 90175 15156 6941747 172836 1609 314 69328 1989 0 102770 174825 0 0 0 0 1 0
   8       1 sda1 90130 15156 6940427 172803 1609 314 69328 1989 0 103673 174792 0 0 0 0 0 0
   9       1 md1p1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0"#;

    #[tokio::test]
    async fn test_disk_io_datasource() {
        let mut reader = HardcodedReader::new();
        reader.add_response(PATH_DISK_STATS, DISK_STATS);

        let ds = DiskIo::new(reader);
        let stats = ds.disk_io().await.unwrap();
        assert_eq!(4, stats.disks.len());

        assert_eq!("nvme0n1", stats.disks[0].device_name);
        assert_eq!(34485575680, stats.disks[0].bytes_written);
        assert_eq!(284154700800, stats.disks[0].bytes_read);
        assert_eq!(1793083, stats.disks[0].write_ops);
        assert_eq!(2745204, stats.disks[0].read_ops);

        assert_eq!("nvme0n1p1", stats.disks[1].device_name);
        assert_eq!(34485575680, stats.disks[1].bytes_written);
        assert_eq!(284153189376, stats.disks[1].bytes_read);
        assert_eq!(1793083, stats.disks[1].write_ops);
        assert_eq!(2745099, stats.disks[1].read_ops);

        assert_eq!("sda", stats.disks[2].device_name);
        assert_eq!(35495936, stats.disks[2].bytes_written);
        assert_eq!(3554174464, stats.disks[2].bytes_read);
        assert_eq!(1609, stats.disks[2].write_ops);
        assert_eq!(90175, stats.disks[2].read_ops);

        assert_eq!("sda1", stats.disks[3].device_name);
        assert_eq!(35495936, stats.disks[3].bytes_written);
        assert_eq!(3553498624, stats.disks[3].bytes_read);
        assert_eq!(1609, stats.disks[3].write_ops);
        assert_eq!(90130, stats.disks[3].read_ops);
    }
}
