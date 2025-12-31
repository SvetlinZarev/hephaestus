use crate::config::Configuration;
use crate::domain::Collector;
use axum::extract::FromRef;
use prometheus::Registry;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub configuration: Arc<Configuration>,
    pub registry: Registry,
    pub collectors: Arc<Vec<Box<dyn Collector>>>,
}

impl FromRef<AppState> for Arc<Vec<Box<dyn Collector>>> {
    fn from_ref(state: &AppState) -> Self {
        state.collectors.clone()
    }
}

impl FromRef<AppState> for Registry {
    fn from_ref(state: &AppState) -> Self {
        state.registry.clone()
    }
}
