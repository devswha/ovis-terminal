//! Where the WebSocket listener binds: the loopback interface unless exposed on purpose.

use std::net::{Ipv4Addr, SocketAddr};

use tokio::net::{TcpListener, TcpStream};

use super::WsError;

/// The address a WebSocket listener binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WsBinding {
    address: SocketAddr,
}

impl WsBinding {
    /// `127.0.0.1:port`, reachable from this host only; port 0 picks a free port.
    #[must_use]
    pub const fn loopback(port: u16) -> Self {
        Self {
            address: SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        }
    }

    /// Any address, including ones other hosts can reach; an explicit choice, never a
    /// default.
    #[must_use]
    pub const fn exposed(address: SocketAddr) -> Self {
        Self { address }
    }

    /// The configured address.
    #[must_use]
    pub const fn address(self) -> SocketAddr {
        self.address
    }
}

impl Default for WsBinding {
    fn default() -> Self {
        Self::loopback(0)
    }
}

/// A bound WebSocket listener.
#[derive(Debug)]
pub struct WsEndpoint {
    listener: TcpListener,
}

impl WsEndpoint {
    /// Binds `binding`.
    ///
    /// # Errors
    ///
    /// [`WsError::Bind`] when the address cannot be bound.
    pub async fn bind(binding: WsBinding) -> Result<Self, WsError> {
        let listener = TcpListener::bind(binding.address)
            .await
            .map_err(WsError::Bind)?;
        Ok(Self { listener })
    }

    /// The bound address, with the port the operating system chose for port 0.
    ///
    /// # Errors
    ///
    /// [`WsError::Bind`] when the socket cannot report its address.
    pub fn local_addr(&self) -> Result<SocketAddr, WsError> {
        self.listener.local_addr().map_err(WsError::Bind)
    }

    /// Accepts one TCP connection.
    ///
    /// # Cancel Safety
    ///
    /// Cancel safe: a dropped call accepts nothing.
    pub(super) async fn accept(&self) -> Result<TcpStream, WsError> {
        let (stream, _) = self.listener.accept().await.map_err(WsError::Accept)?;
        // Replies are whole records; waiting to coalesce them only adds latency.
        stream.set_nodelay(true).map_err(WsError::Accept)?;
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::{WsBinding, WsEndpoint};

    #[tokio::test]
    async fn the_default_binding_is_reachable_from_this_host_only() -> Result<(), super::WsError> {
        let endpoint = WsEndpoint::bind(WsBinding::default()).await?;
        let bound = endpoint.local_addr()?;
        assert!(bound.ip().is_loopback(), "{bound}");
        assert_ne!(bound.port(), 0, "port 0 resolves to a real port");
        let exposed = WsBinding::exposed(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0));
        assert!(!exposed.address().ip().is_loopback());
        Ok(())
    }
}
