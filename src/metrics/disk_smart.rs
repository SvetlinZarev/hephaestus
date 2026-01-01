use crate::domain::{Collector, Metric};
use crate::metrics::no_operation::NoOpCollector;
use crate::metrics::util::{into_labels, maybe_counter, maybe_gauge};
use prometheus::Registry;
use prometheus::core::Desc;
use prometheus::proto::{LabelPair, MetricFamily};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone)]
pub struct SmartReports {
    pub timestamp: Instant,
    pub sata: Vec<SataDevice>,
    pub nvme: Vec<NvmeDevice>,
}

#[derive(Debug, Clone)]
pub struct SataDevice {
    pub device: Device,
    pub temperature: Option<f64>,
    pub temperature_min: Option<f64>,
    pub temperature_max: Option<f64>,
    pub start_stop_count: Option<u64>,
    pub power_on_hours: Option<u64>,
    pub power_cycle_count: Option<u64>,
    pub load_cycle_count: Option<u64>,
    pub reallocated_sectors: Option<u64>,
    pub pending_sectors: Option<u64>,
    pub uncorrectable_errors: Option<u64>,
    pub crc_errors: Option<u64>,
    pub wear_level: Option<f64>,
}

impl SataDevice {
    pub fn new(device: Device) -> Self {
        Self {
            device,
            temperature: None,
            temperature_min: None,
            temperature_max: None,
            start_stop_count: None,
            power_on_hours: None,
            power_cycle_count: None,
            load_cycle_count: None,
            reallocated_sectors: None,
            pending_sectors: None,
            uncorrectable_errors: None,
            crc_errors: None,
            wear_level: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NvmeDevice {
    pub device: Device,
    pub temperature: Option<f64>,
    pub available_spare: Option<f64>,
    pub percent_used: Option<f64>,
    pub data_units_read: Option<u64>,
    pub data_units_written: Option<u64>,
    pub host_reads: Option<u64>,
    pub host_writes: Option<u64>,
    pub power_on_hours: Option<u64>,
    pub unsafe_shutdowns: Option<u64>,
    pub media_errors: Option<u64>,
}

impl NvmeDevice {
    pub fn new(device: Device) -> Self {
        Self {
            device,
            temperature: None,
            available_spare: None,
            percent_used: None,
            data_units_read: None,
            data_units_written: None,
            host_reads: None,
            host_writes: None,
            power_on_hours: None,
            unsafe_shutdowns: None,
            media_errors: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Device {
    pub device: String,
    pub model: String,
    pub serial_number: String,
}

pub trait DataSource {
    fn disk_temps(&self) -> impl Future<Output = anyhow::Result<SmartReports>> + Send;
}

#[derive(Clone)]
pub struct Metrics {
    state: Arc<Mutex<Option<SmartReports>>>,

    sata_temp: Desc,
    sata_temp_min: Desc,
    sata_temp_max: Desc,
    sata_start_stop: Desc,
    sata_power_on: Desc,
    sata_power_cycle: Desc,
    sata_load_cycle: Desc,
    sata_reallocated: Desc,
    sata_pending: Desc,
    sata_uncorrectable: Desc,
    sata_crc_errors: Desc,
    sata_wear_level: Desc,

    nvme_temp: Desc,
    nvme_available_spare: Desc,
    nvme_percent_used: Desc,
    nvme_data_read: Desc,
    nvme_data_written: Desc,
    nvme_host_reads: Desc,
    nvme_host_writes: Desc,
    nvme_power_on: Desc,
    nvme_unsafe_shutdowns: Desc,
    nvme_media_errors: Desc,
}

impl Metrics {
    pub fn new(state: Arc<Mutex<Option<SmartReports>>>) -> anyhow::Result<Self> {
        let labels = vec!["device".into(), "model".into(), "serial_number".into()];

        Ok(Self {
            state,

            // --- SATA Descriptors ---
            sata_temp: Desc::new(
                "system_smart_sata_temperature_celsius".into(),
                "Current SATA disk temperature".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_temp_min: Desc::new(
                "smart_sata_temperature_min_celsius".into(),
                "Minimum temperature recorded by the SATA device".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_temp_max: Desc::new(
                "smart_sata_temperature_max_celsius".into(),
                "Maximum temperature recorded by the SATA device".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_start_stop: Desc::new(
                "system_smart_sata_start_stop_count_total".into(),
                "Total SATA start/stop cycles".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_power_on: Desc::new(
                "system_smart_sata_power_on_hours_total".into(),
                "Total SATA power on hours".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_power_cycle: Desc::new(
                "system_smart_sata_power_cycle_count_total".into(),
                "Total SATA power cycles".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_load_cycle: Desc::new(
                "system_smart_sata_load_cycle_count_total".into(),
                "Total SATA load/unload cycles".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_reallocated: Desc::new(
                "system_smart_sata_reallocated_sectors_total".into(),
                "Total SATA reallocated sectors count".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_pending: Desc::new(
                "system_smart_sata_pending_sectors_total".into(),
                "Total SATA pending sectors count".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_uncorrectable: Desc::new(
                "system_smart_sata_uncorrectable_errors_total".into(),
                "Total SATA uncorrectable errors count".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_crc_errors: Desc::new(
                "system_smart_sata_crc_errors_total".into(),
                "Total SATA interface CRC errors (UDMA_CRC_Error_Count)".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            sata_wear_level: Desc::new(
                "system_smart_sata_wear_level_ratio".into(),
                "SATA SSD wear level (1.0 is new, 0.0 is end of life)".into(),
                labels.clone(),
                HashMap::new(),
            )?,

            // --- NVMe Descriptors ---
            nvme_temp: Desc::new(
                "system_smart_nvme_temperature_celsius".into(),
                "Current NVMe disk temperature".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nvme_available_spare: Desc::new(
                "system_smart_nvme_available_spare_ratio".into(),
                "NVMe remaining spare capacity ratio (0-1)".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nvme_percent_used: Desc::new(
                "system_smart_nvme_percent_used_ratio".into(),
                "NVMe life used ratio (0-1, can exceed 1)".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nvme_data_read: Desc::new(
                "system_smart_nvme_data_units_read_total".into(),
                "Total NVMe data units read (512 byte units)".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nvme_data_written: Desc::new(
                "system_smart_nvme_data_units_written_total".into(),
                "Total NVMe data units written (512 byte units)".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nvme_host_reads: Desc::new(
                "system_smart_nvme_host_reads_total".into(),
                "Total NVMe host read commands".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nvme_host_writes: Desc::new(
                "system_smart_nvme_host_writes_total".into(),
                "Total NVMe host write commands".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nvme_power_on: Desc::new(
                "system_smart_nvme_power_on_hours_total".into(),
                "Total NVMe power on hours".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nvme_unsafe_shutdowns: Desc::new(
                "system_smart_nvme_unsafe_shutdowns_total".into(),
                "Total NVMe unsafe shutdowns".into(),
                labels.clone(),
                HashMap::new(),
            )?,
            nvme_media_errors: Desc::new(
                "system_smart_nvme_media_errors_total".into(),
                "Total NVMe media and data integrity errors".into(),
                labels.clone(),
                HashMap::new(),
            )?,
        })
    }

    pub fn register(&self, registry: &Registry) -> anyhow::Result<()> {
        registry.register(Box::new(self.clone()))?;
        Ok(())
    }

    fn make_labels(&self, device: &Device) -> Vec<LabelPair> {
        into_labels(&[
            ("device", &device.device),
            ("model", &device.model),
            ("serial_number", &device.serial_number),
        ])
    }
}

impl prometheus::core::Collector for Metrics {
    fn desc(&self) -> Vec<&Desc> {
        vec![
            &self.sata_temp,
            &self.sata_temp_min,
            &self.sata_temp_max,
            &self.sata_start_stop,
            &self.sata_power_on,
            &self.sata_power_cycle,
            &self.sata_load_cycle,
            &self.sata_reallocated,
            &self.sata_pending,
            &self.sata_uncorrectable,
            &self.sata_crc_errors,
            &self.sata_wear_level,
            &self.nvme_temp,
            &self.nvme_available_spare,
            &self.nvme_percent_used,
            &self.nvme_data_read,
            &self.nvme_data_written,
            &self.nvme_host_reads,
            &self.nvme_host_writes,
            &self.nvme_power_on,
            &self.nvme_unsafe_shutdowns,
            &self.nvme_media_errors,
        ]
    }

    fn collect(&self) -> Vec<MetricFamily> {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stats) = guard.as_ref() else {
            return vec![];
        };

        let mut families = Vec::new();

        for n in &stats.nvme {
            let l = self.make_labels(&n.device);
            let f = &mut families;

            maybe_gauge(f, &self.nvme_temp, &l, n.temperature);
            maybe_gauge(f, &self.nvme_available_spare, &l, n.available_spare);
            maybe_gauge(f, &self.nvme_percent_used, &l, n.percent_used);
            maybe_counter(f, &self.nvme_data_read, &l, n.data_units_read);
            maybe_counter(f, &self.nvme_data_written, &l, n.data_units_written);
            maybe_counter(f, &self.nvme_host_reads, &l, n.host_reads);
            maybe_counter(f, &self.nvme_host_writes, &l, n.host_writes);
            maybe_counter(f, &self.nvme_power_on, &l, n.power_on_hours);
            maybe_counter(f, &self.nvme_unsafe_shutdowns, &l, n.unsafe_shutdowns);
            maybe_counter(f, &self.nvme_media_errors, &l, n.media_errors);
        }

        for s in &stats.sata {
            let l = self.make_labels(&s.device);
            let f = &mut families;

            maybe_gauge(f, &self.sata_temp, &l, s.temperature);
            maybe_gauge(f, &self.sata_temp_min, &l, s.temperature_min);
            maybe_gauge(f, &self.sata_temp_max, &l, s.temperature_max);
            maybe_gauge(f, &self.sata_pending, &l, s.pending_sectors);
            maybe_gauge(f, &self.sata_reallocated, &l, s.reallocated_sectors);
            maybe_gauge(f, &self.sata_wear_level, &l, s.wear_level);
            maybe_counter(f, &self.sata_start_stop, &l, s.start_stop_count);
            maybe_counter(f, &self.sata_power_on, &l, s.power_on_hours);
            maybe_counter(f, &self.sata_power_cycle, &l, s.power_cycle_count);
            maybe_counter(f, &self.sata_load_cycle, &l, s.load_cycle_count);
            maybe_counter(f, &self.sata_uncorrectable, &l, s.uncorrectable_errors);
            maybe_counter(f, &self.sata_crc_errors, &l, s.crc_errors);
        }

        families
    }
}

pub struct Smart {
    config: Config,
}

impl Smart {
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl<T> Metric<T> for Smart
where
    T: DataSource + Send + Sync + 'static,
{
    fn register(self, registry: &Registry, data_source: T) -> anyhow::Result<Box<dyn Collector>> {
        if !self.config.enabled {
            return Ok(Box::new(NoOpCollector::new()));
        }

        let collector = SmartCollector::new(data_source);
        let measurements = collector.measurements();

        let metrics = Metrics::new(measurements)?;
        registry.register(Box::new(metrics))?;

        Ok(Box::new(collector))
    }
}

struct SmartCollector<T> {
    measurements: Arc<Mutex<Option<SmartReports>>>,
    data_source: T,
}

impl<T> SmartCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    fn new(data_source: T) -> Self {
        Self {
            data_source,
            measurements: Arc::new(Mutex::new(None)),
        }
    }

    fn measurements(&self) -> Arc<Mutex<Option<SmartReports>>> {
        self.measurements.clone()
    }
}

#[async_trait::async_trait]
impl<T> Collector for SmartCollector<T>
where
    T: DataSource + Send + Sync + 'static,
{
    async fn collect(&self) -> anyhow::Result<()> {
        let stats = self.data_source.disk_temps().await?;
        *self.measurements.lock().unwrap_or_else(|e| e.into_inner()) = Some(stats);
        Ok(())
    }
}
