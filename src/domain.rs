use prometheus::Registry;

pub trait Metric {
    fn register(self, registry: &Registry) -> anyhow::Result<Box<dyn Collector>>;
}

#[async_trait::async_trait]
pub trait Collector: Send + Sync + 'static {
    async fn collect(&self) -> anyhow::Result<()>;
}
