use crate::datasource::Reader;
use crate::metrics::network_io::{DataSource, InterfaceStats, NetworkIoStats};
use tokio::time::Instant;

const PATH_NET_DEV: &str = "/proc/net/dev";

pub struct NetworkIo<R> {
    reader: R,
}

impl<R> NetworkIo<R>
where
    R: Reader,
{
    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R> DataSource for NetworkIo<R>
where
    R: Reader,
{
    async fn network_io(&self) -> anyhow::Result<NetworkIoStats> {
        let content = self.reader.read_to_string(PATH_NET_DEV).await?;
        let timestamp = Instant::now();
        let mut interfaces = Vec::new();

        for line in content.lines().skip(2) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let Some((iface, stats)) = line.split_once(':') else {
                tracing::debug!("Unexpected line while parsing network io stats: {}", line);
                continue;
            };

            let mut stats = stats.split_whitespace();

            let bytes_received = stats.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let packets_received = stats.next().and_then(|v| v.parse().ok()).unwrap_or(0);

            // Skip remaining 6 receive columns (errs, drop, fifo, frame, compressed, multicast)
            stats.nth(5);

            let bytes_sent = stats.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let packets_sent = stats.next().and_then(|v| v.parse().ok()).unwrap_or(0);

            interfaces.push(InterfaceStats {
                interface: iface.trim().to_string(),
                bytes_sent,
                bytes_received,
                packets_sent,
                packets_received,
            });
        }

        Ok(NetworkIoStats {
            timestamp,
            interfaces,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::datasource::network_io::{NetworkIo, PATH_NET_DEV};
    use crate::datasource::tests::HardcodedReader;
    use crate::metrics::network_io::DataSource;

    const NET_DEV_TEXT: &str = r#"Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 5467426526  298140    0    0    0     0          0         0 5467426526  298140    0    0    0     0       0          0
enp1s0: 23258276045 17679116    0 11185    0     0          0     56575 56878436846 2548501    0    0    0     0       0          0
wlp2s0:       0       0    0    0    0     0          0         0        0       0    0    0    0     0       0          0
"#;

    #[tokio::test]
    async fn test_network_io_datasource() {
        let mut reader = HardcodedReader::new();
        reader.add_response(PATH_NET_DEV, NET_DEV_TEXT);

        let ds = NetworkIo::new(reader);
        let nio = ds
            .network_io()
            .await
            .expect("Failed to read network IO usage statistics");
        assert_eq!(nio.interfaces.len(), 3);

        assert_eq!(nio.interfaces[0].interface, "lo");
        assert_eq!(nio.interfaces[0].bytes_received, 5467426526);
        assert_eq!(nio.interfaces[0].packets_received, 298140);
        assert_eq!(nio.interfaces[0].bytes_sent, 5467426526);
        assert_eq!(nio.interfaces[0].packets_sent, 298140);

        assert_eq!(nio.interfaces[1].interface, "enp1s0");
        assert_eq!(nio.interfaces[1].bytes_received, 23258276045);
        assert_eq!(nio.interfaces[1].packets_received, 17679116);
        assert_eq!(nio.interfaces[1].bytes_sent, 56878436846);
        assert_eq!(nio.interfaces[1].packets_sent, 2548501);

        assert_eq!(nio.interfaces[2].interface, "wlp2s0");
        assert_eq!(nio.interfaces[2].bytes_received, 0);
        assert_eq!(nio.interfaces[2].packets_received, 0);
        assert_eq!(nio.interfaces[2].bytes_sent, 0);
        assert_eq!(nio.interfaces[2].packets_sent, 0);
    }
}
