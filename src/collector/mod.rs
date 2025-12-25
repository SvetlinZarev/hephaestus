use prometheus::Registry;

pub mod cpu_usage;
pub mod cpu_frequency;

#[async_trait::async_trait]
pub trait Metric: Send + Sync {
    fn name(&self) -> &'static str;

    fn enabled(&self) -> bool;

    async fn supported(&self) -> bool;

    fn register(&self, registry: &Registry) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait Collector: Send + Sync {
    async fn collect(&self) -> anyhow::Result<()>;
}
