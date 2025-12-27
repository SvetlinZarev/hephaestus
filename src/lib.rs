#![deny(unsafe_code)]
//#![deny(warnings)]
#![deny(unused_must_use)]
#![deny(unexpected_cfgs)]
extern crate core;

pub mod domain;
pub mod bootstrap;
pub mod config;
pub mod logging;
pub mod metrics;
pub mod server;
pub(crate) mod datasource;
