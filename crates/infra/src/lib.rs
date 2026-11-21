//! Operational code shared by server processes: config loading, logging, metrics, admin port.
//!
//! Game rules and the network protocol do not belong here. Depends on no other workspace crate.

pub mod admin;
pub mod config;
pub mod hex;
pub mod http;
pub mod logging;
pub mod process;
pub mod ready;
pub mod runtime;
pub mod server;
pub mod shutdown;
