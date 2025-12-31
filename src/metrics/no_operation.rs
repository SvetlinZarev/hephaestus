use crate::domain::Collector;

#[derive(Default)]
pub struct NoOpCollector {
    //
}

impl NoOpCollector {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl Collector for NoOpCollector {
    async fn collect(&self) -> anyhow::Result<()> {
        // do nothing by design
        Ok(())
    }
}
