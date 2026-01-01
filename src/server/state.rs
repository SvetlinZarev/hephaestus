use crate::config::Configuration;
use crate::domain::Collector;
use prometheus::Registry;
use std::ops::Deref;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::Instant;

#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<Inner>,
}

impl Deref for AppState {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub struct Inner {
    pub configuration: Configuration,
    pub registry: Registry,
    pub collectors: Vec<Box<dyn Collector>>,
    pub last_collection: Mutex<Instant>,
}
