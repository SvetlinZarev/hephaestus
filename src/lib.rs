#![deny(unsafe_code)]
#![deny(warnings)]
#![deny(unused_must_use)]
#![deny(unexpected_cfgs)]
pub mod bootstrap;
pub mod config;
pub(crate) mod datasource;
pub mod domain;
pub mod logging;
pub mod metrics;
pub mod server;
