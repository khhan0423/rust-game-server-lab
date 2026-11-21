//! Named HTTP listeners with bind-time access policy and graceful shutdown.

pub mod api;

use std::{
    io,
    net::{IpAddr, SocketAddr},
};

use axum::Router;
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::shutdown::Shutdown;

/// Address of an HTTP service in its TOML section.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpConfig {
    listen_addr: SocketAddr,
}

impl HttpConfig {
    /// Configures a listener at the given address, allowing port zero.
    pub const fn new(listen_addr: SocketAddr) -> Self {
        Self { listen_addr }
    }
}

/// Network permitted to reach a listener.
#[derive(Clone, Copy, Debug)]
pub enum Exposure {
    /// Local operator endpoint.
    Loopback,
    /// Admin endpoint on loopback or a private network.
    Private,
    /// Public application endpoint.
    Public,
}

impl Exposure {
    fn permits(self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        match self {
            Self::Public => true,
            Self::Loopback => ip.is_loopback(),
            Self::Private => {
                ip.is_loopback()
                    || match ip {
                        IpAddr::V4(ip) => ip.is_private(),
                        IpAddr::V6(ip) => ip.is_unique_local(),
                    }
            }
        }
    }
}

/// Owns a bound HTTP socket until its requests have drained.
#[derive(Debug)]
pub struct HttpListener {
    name: &'static str,
    addr: SocketAddr,
    socket: TcpListener,
}

impl HttpListener {
    /// Checks the address against the exposure policy before binding.
    pub async fn bind(
        name: &'static str,
        config: &HttpConfig,
        exposure: Exposure,
    ) -> io::Result<Self> {
        if !exposure.permits(config.listen_addr.ip()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{name} address violates {exposure:?} exposure"),
            ));
        }
        let socket = TcpListener::bind(config.listen_addr).await?;
        Ok(Self {
            name,
            addr: socket.local_addr()?,
            socket,
        })
    }

    /// Name used in readiness and logs.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Actual address assigned by the operating system.
    pub const fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Serves the router and drains pending requests when shutdown begins.
    pub async fn serve(self, app: Router, shutdown: Shutdown) -> io::Result<()> {
        axum::serve(self.socket, app)
            .with_graceful_shutdown(shutdown.wait())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_releases_the_bound_port() -> io::Result<()> {
        let config = HttpConfig::new(SocketAddr::from(([127, 0, 0, 1], 0)));
        let listener = HttpListener::bind("test", &config, Exposure::Loopback).await?;
        let address = listener.local_addr();
        let shutdown = Shutdown::default();
        let task = tokio::spawn(listener.serve(Router::new(), shutdown.clone()));
        shutdown.cancel();
        task.await??;
        let _rebound = TcpListener::bind(address).await?;
        Ok(())
    }
}
